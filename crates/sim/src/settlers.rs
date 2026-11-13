//! The first settlers (B39/D62, made visible in B40): a party walks in from the
//! map edge, camps, fells trees for timber and builds the first log cabins — and
//! the first lanes only appear once the paths they walk have worn into cart
//! tracks.
//!
//! Movement is *analytic*: a member holds a leg (`from`, `to`, `t0`, `dur`) and
//! its position is interpolated from the sim clock. The planner ticks in 20 s
//! steps, so integrating a position per step made walkers jump five tiles at a
//! time and overshoot every short approach (B39: a chopper bounced between two
//! points for ever and felled exactly one tree). Interpolating fixes both: the
//! wallpaper's frame clock (`Sim::smooth_time`) moves the figures smoothly
//! between planning steps.

use crate::{LotKind, Sim, Stage};

/// The planning tick while the settlement is on screen (B40): fine enough that
/// a walker moves smoothly, a chop swings and a tree going over is visible.
pub const SETTLE_STEP: f64 = 2.0;
/// Walking speed for short work trips, in tiles per sim second.
pub const WALK: f32 = 0.22;
/// The walk-in is a fixed, *visible* length of time: ~3.5 minutes at month pace,
/// whatever the distance from the edge (B40: the party has to be watchable).
pub const WALK_IN_SECONDS: f64 = 210.0;
/// Logs one tree yields, and the logs the first cabin needs.
pub const LOGS_PER_TREE: u32 = 3;
pub const LOGS_PER_CABIN: u32 = 9;
/// How long a chopper works one tree, in sim seconds.
pub const CHOP_SECONDS: f64 = 80.0;
/// How long a felled tree takes to go over.
pub const FALL_SECONDS: f64 = 3.0;
/// How long a carrier takes to load logs at a pile.
pub const LOAD_SECONDS: f64 = 5.0;
/// The settlement hands over to the village planner after this many cabins.
pub const CABINS_TO_SETTLE: u32 = 4;

/// One member of the party.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// 0 = settler, 1 = the wagon, 2 = a chopper, 3 = a log carrier.
    pub kind: u8,
    pub from: (f32, f32),
    pub to: (f32, f32),
    /// Sim time this leg started, and how long it takes.
    pub t0: f64,
    pub dur: f64,
    /// 0 = walking, 1 = chopping, 2 = loading.
    pub state: u8,
    /// When the current job finishes.
    pub busy_until: f64,
    /// The tree being chopped or the pile being loaded.
    pub job: (i32, i32),
    /// Logs in hand.
    pub load: u32,
}

impl Member {
    fn at(&self, t: f64) -> (f32, f32) {
        if self.dur <= 0.0 || t >= self.t0 + self.dur {
            return self.to;
        }
        let f = (((t - self.t0) / self.dur) as f32).clamp(0.0, 1.0);
        (
            self.from.0 + (self.to.0 - self.from.0) * f,
            self.from.1 + (self.to.1 - self.from.1) * f,
        )
    }

    /// Start a new leg from wherever this member is at `t`.
    fn go(&mut self, to: (f32, f32), t: f64, speed: f32) {
        let from = self.at(t);
        let d = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        self.from = from;
        self.to = to;
        self.t0 = t;
        self.dur = (d / speed.max(0.01)) as f64;
        self.state = 1; // walking
        self.busy_until = 0.0;
    }
}

/// The camp on the founding spot.
#[derive(Clone, Debug, PartialEq)]
pub struct Camp {
    /// Tent tiles, relative to the founding spot.
    pub tents: Vec<(i32, i32)>,
    /// The campfire.
    pub fire: (i32, i32),
    /// When the fire was lit (sim seconds), for the smoke/glow.
    pub lit: f64,
    /// Is the fire on ground the settlement still holds? (B41b: a fire on a
    /// paved square is not drawn.)
    pub fire_ok: bool,
}

/// A log pile or a stump left by the felling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogPile {
    pub x: i32,
    pub y: i32,
    /// 0 = log pile, 1 = stump.
    pub kind: u8,
}

/// A tree on its way down (B40): drawn leaning over `FALL_SECONDS`, then it
/// becomes a stump and a pile of logs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falling {
    pub x: i32,
    pub y: i32,
    pub t0: f64,
    pub dur: f64,
    /// Which way it tips: -1 or +1 across the screen.
    pub dir: i8,
    pub size: u8,
    pub variant: u8,
}

/// The whole settlement phase.
#[derive(Clone, Debug, Default)]
pub struct Settlers {
    pub party: Vec<Member>,
    pub camp: Option<Camp>,
    /// Where the party is walking to (the founding spot).
    pub target: (i32, i32),
    pub arrived: bool,
    pub arrived_at: f64,
    /// Timber in hand (logs cut but not yet used).
    pub timber: u32,
    /// Piles and stumps on the map.
    pub piles: Vec<LogPile>,
    /// Trees currently going over.
    pub falling: Vec<Falling>,
    /// Trees felled so far.
    pub felled: u32,
    /// Cabins staked so far.
    pub cabins: u32,
    /// The day the first cabin was finished, for the handback.
    pub first_cabin_day: Option<f64>,
    /// Stumps pulled, and the first cart track laid.
    pub stumps_pulled: bool,
    pub road_laid: bool,
    /// The camp is struck: tents down, stumps out, logs used up, the party
    /// gone. Nothing of the settlement draws after this (B41b).
    pub struck: bool,
    /// Tiles the party wore a trail into, faded when the camp is struck.
    pub worn_trail: std::collections::BTreeSet<(i32, i32)>,
}

impl Settlers {
    pub fn is_active(&self) -> bool {
        !self.struck && (!self.arrived || self.cabins < CABINS_TO_SETTLE)
    }
}

impl Sim {
    /// Spawn the party at the map edge, on the river/valley side, walking to the
    /// founding spot (B39 item 1). No stakes, no lane.
    pub(crate) fn spawn_party(&mut self) {
        let (fx, fy) = self.founding;
        let (w, h) = (self.map.w, self.map.h);
        // Walk in from the nearest edge of the map: pick the edge with the most
        // gentle ground between it and the camp (a valley, not a cliff).
        let mut best: Option<(f32, (i32, i32))> = None;
        for (ex, ey) in [(6, fy), (w - 7, fy), (fx, 6), (fx, h - 7)] {
            if !self.map.in_bounds(ex, ey) {
                continue;
            }
            let d = (((ex - fx).pow(2) + (ey - fy).pow(2)) as f32).sqrt();
            if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
                best = Some((d, (ex, ey)));
            }
        }
        let (sx, sy) = best.map(|(_, p)| p).unwrap_or((6, fy));
        let dist = (((sx - fx).pow(2) + (sy - fy).pow(2)) as f32).sqrt().max(1.0);
        // A fixed walk-in time, so the party is always watchable (B40).
        let dur = (dist as f64 / WALK as f64).clamp(WALK_IN_SECONDS * 0.6, WALK_IN_SECONDS);
        let mut rng = self.rng(0x5E77_1E12);
        let mut party = Vec::new();
        // 4-8 settlers plus the wagon, in a loose column behind the leader.
        let n = 4 + rng.usize(5);
        // The party walks in single file along one route (B40): a column spread
        // across the meadow wore a bundle of parallel hairlines, not a path.
        let (dx, dy) = (fx - sx, fy - sy);
        let l = ((dx * dx + dy * dy) as f32).sqrt().max(1.0);
        let (ux, uy) = (dx as f32 / l, dy as f32 / l);
        let (px_, py_) = (-uy, ux);
        for i in 0..n {
            let back = i as f32 * 1.9;
            let side = if i % 2 == 0 { -0.25 } else { 0.25 };
            let from = (
                sx as f32 + 0.5 - ux * back + px_ * side,
                sy as f32 + 0.5 - uy * back + py_ * side,
            );
            let to = (
                fx as f32 + 0.5 - ux * back * 0.55 + px_ * side,
                fy as f32 + 1.4 - uy * back * 0.55 + py_ * side,
            );
            party.push(Member {
                kind: 0,
                from,
                to,
                t0: 0.0,
                dur: dur + i as f64 * 1.5,
                state: 0,
                busy_until: 0.0,
                job: (0, 0),
                load: 0,
            });
        }
        // The wagon follows the party, a little slower.
        party.push(Member {
            kind: 1,
            from: (sx as f32 + 0.5 + 1.5, sy as f32 + 2.0),
            to: (fx as f32 + 2.0, fy as f32 + 2.6),
            t0: 0.0,
            dur: dur + 12.0,
            state: 0,
            busy_until: 0.0,
            job: (0, 0),
            load: 0,
        });
        self.settlers = Settlers {
            party,
            camp: None,
            target: (fx, fy),
            arrived: false,
            arrived_at: 0.0,
            timber: 0,
            piles: Vec::new(),
            falling: Vec::new(),
            felled: 0,
            cabins: 0,
            first_cabin_day: None,
            stumps_pulled: false,
            road_laid: false,
            struck: false,
            worn_trail: std::collections::BTreeSet::new(),
        };
        self.events.push(crate::Event {
            time: 0.0,
            text: format!("a party of {n} settlers and a wagon come up the valley"),
        });
    }

    /// One planning step of the settlement phase (B39 items 1-3).
    pub(crate) fn settlers_step(&mut self) {
        if self.settlers.party.is_empty() {
            return;
        }
        let now = self.time;
        // --- 1. the walk in ------------------------------------------------
        if !self.settlers.arrived {
            self.wear_along_legs(now);
            if self.settlers.party.iter().all(|m| now >= m.t0 + m.dur) {
                self.settlers.arrived = true;
                self.settlers.arrived_at = now;
                let (fx, fy) = self.founding;
                self.settlers.camp = Some(Camp {
                    tents: vec![(-2, 0), (2, 0), (0, 2)],
                    fire: (0, 0),
                    lit: now,
                    fire_ok: true,
                });
                // Four workers: two choppers, two carriers.
                self.settlers.party = (0..4)
                    .map(|i| {
                        let kind = if i % 2 == 0 { 2 } else { 3 };
                        let off = (i / 2) as f32;
                        Member {
                            kind,
                            from: (fx as f32 + 2.5 + off, fy as f32 + 2.5),
                            to: (fx as f32 + 2.5 + off, fy as f32 + 2.5),
                            t0: now,
                            dur: 0.0,
                            state: 0,
                            busy_until: 0.0,
                            job: (0, 0),
                            load: 0,
                        }
                    })
                    .collect();
                self.events.push(crate::Event {
                    time: now,
                    text: "the settlers pitch their tents and light a fire".to_string(),
                });
            }
            return;
        }

        // --- 3. felling, carrying and the first cabins ----------------------
        self.wear_along_legs(now);
        // The trees that went over since the last step become a stump and a pile.
        let mut landed: Vec<(i32, i32)> = Vec::new();
        self.settlers.falling.retain(|f| {
            if now + 1e-9 >= f.t0 + f.dur {
                landed.push((f.x, f.y));
                false
            } else {
                true
            }
        });
        for (x, y) in landed {
            self.settlers.piles.push(LogPile { x, y, kind: 1 });
            // The pile of logs sits beside the stump, on free ground.
            let (px, py) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .map(|(dx, dy)| (x + dx, y + dy))
                .find(|&(px, py)| self.map.in_bounds(px, py) && !self.map.at(px, py).kind.is_lane())
                .unwrap_or((x, y + 1));
            self.settlers.piles.push(LogPile { x: px, y: py, kind: 0 });
        }

        let (fx, fy) = self.founding;
        let settled = self.settlers.cabins >= CABINS_TO_SETTLE;
        let need_timber = self.settlers.timber < LOGS_PER_CABIN * 2;

        // (a) legs that finished, and jobs that finished.
        for i in 0..self.settlers.party.len() {
            let now_i = now;
            let m = &mut self.settlers.party[i];
            match m.state {
                // Walking: arrived.
                1 if now_i + 1e-9 >= m.t0 + m.dur => {
                    m.state = 0;
                    m.dur = 0.0;
                }
                // Chopping: the tree goes over.
                2 if now_i + 1e-9 >= m.busy_until => {
                    m.state = 0;
                    m.dur = 0.0;
                }
                // Loading a pile: the logs are in hand.
                3 if now_i + 1e-9 >= m.busy_until => {
                    m.state = 0;
                    m.dur = 0.0;
                }
                _ => {}
            }
        }
        // A chopper that has finished its chop fells the tree it stood at.
        let mut felled: Vec<(i32, i32)> = Vec::new();
        for i in 0..self.settlers.party.len() {
            let m = &self.settlers.party[i];
            if m.kind == 2 && m.state == 0 && m.dur == 0.0 && m.busy_until > 0.0 {
                let (x, y) = m.job;
                if self.map.at(x, y).prop.is_tree() {
                    felled.push((x, y));
                }
                self.settlers.party[i].busy_until = 0.0;
            }
        }
        for (x, y) in felled {
            self.fell_tree(x, y, now);
        }
        // A carrier that has arrived at a pile picks the logs up.
        for i in 0..self.settlers.party.len() {
            let (kind, state, job) = {
                let m = &self.settlers.party[i];
                (m.kind, m.state, m.job)
            };
            if kind == 3 && state == 0 && self.settlers.party[i].load == 0 {
                if let Some(k) = self
                    .settlers
                    .piles
                    .iter()
                    .position(|p| p.kind == 0 && (p.x, p.y) == job)
                {
                    let here = self.settlers.party[i].at(now);
                    let (dx, dy) = (here.0 - job.0 as f32, here.1 - job.1 as f32);
                    if dx * dx + dy * dy < 4.0 {
                        self.settlers.piles.remove(k);
                        self.settlers.party[i].load = LOGS_PER_TREE;
                        self.settlers.party[i].state = 3;
                        self.settlers.party[i].busy_until = now + LOAD_SECONDS;
                    }
                }
            }
        }
        // A carrier back at the camp drops its logs.
        for i in 0..self.settlers.party.len() {
            let m = &self.settlers.party[i];
            if m.kind == 3 && m.state == 0 && m.load > 0 {
                let here = m.at(now);
                let (dx, dy) = (here.0 - (fx as f32 + 3.0), here.1 - (fy as f32 + 3.0));
                if dx * dx + dy * dy < 6.0 {
                    let load = m.load;
                    self.settlers.timber += load;
                    self.settlers.party[i].load = 0;
                }
            }
        }

        // (b) idle workers pick the next job: the nearest unclaimed tree
        // (choppers) or the nearest pile (carriers). Only what is needed — no
        // clear-cut.
        let claimed: Vec<(i32, i32)> = self
            .settlers
            .party
            .iter()
            .filter(|m| m.kind == 2 && (m.state == 2 || m.state == 1))
            .map(|m| m.job)
            .collect();
        for i in 0..self.settlers.party.len() {
            let m = &self.settlers.party[i];
            if m.state != 0 || m.dur != 0.0 {
                continue;
            }
            let here = m.at(now);
            match m.kind {
                2 if need_timber && !settled => {
                    if let Some((x, y)) = self.nearest_tree(here, 14, &claimed) {
                        let (dx, dy) = (x as f32 + 0.5 - here.0, y as f32 + 0.5 - here.1);
                        if dx * dx + dy * dy <= 2.0 {
                            // Close enough to swing: chop where it stands.
                            self.settlers.party[i].job = (x, y);
                            self.settlers.party[i].state = 2;
                            self.settlers.party[i].busy_until = now + CHOP_SECONDS;
                        } else {
                            self.settlers.party[i].job = (x, y);
                            self.settlers.party[i].go((x as f32 + 0.5, y as f32 + 0.5), now, WALK);
                        }
                    }
                }
                3 if m.load > 0 => {
                    self.settlers.party[i].go((fx as f32 + 3.0, fy as f32 + 3.0), now, WALK);
                }
                3 => {
                    if let Some((px, py)) = self
                        .settlers
                        .piles
                        .iter()
                        .filter(|p| p.kind == 0)
                        .min_by_key(|p| {
                            let (dx, dy) = (p.x as f32 - here.0, p.y as f32 - here.1);
                            (dx * dx + dy * dy) as i32
                        })
                        .map(|p| (p.x, p.y))
                    {
                        self.settlers.party[i].job = (px, py);
                        self.settlers.party[i].go((px as f32 + 0.5, py as f32 + 0.5), now, WALK);
                    } else if (here.0 - fx as f32).abs() > 3.0 || (here.1 - fy as f32).abs() > 3.0
                    {
                        // Nothing to carry: come back to the fire.
                        self.settlers.party[i].go((fx as f32 + 3.0, fy as f32 + 3.0), now, WALK);
                    }
                }
                _ => {}
            }
        }

        // Timber in hand stakes the next cabin (B39 item 3).
        while self.settlers.timber >= LOGS_PER_CABIN && !settled {
            if !self.stake_cabin() {
                break;
            }
            self.settlers.timber -= LOGS_PER_CABIN;
            self.settlers.cabins += 1;
            if self.settlers.cabins == 1 {
                self.settlers.first_cabin_day = Some(self.days());
            }
        }

        // --- the camp ends with the settlement (B41b) ----------------------
        // The tents come down once the cabins are roofed and the village has
        // its first proper house; the stumps are pulled and the log piles used
        // up at the same moment, so nothing of the camp outlives the era it
        // belongs to. A save from an older build (a camp with a grown city
        // around it) is struck on its first step.
        let cabins_roofed = self.settlers.cabins >= CABINS_TO_SETTLE
            && self
                .lots
                .iter()
                .filter(|l| l.kind == LotKind::LogCabin)
                .all(|l| l.finished());
        let village_house = self
            .lots
            .iter()
            .any(|l| !matches!(l.kind, LotKind::LogCabin) && l.finished());
        if cabins_roofed && village_house || self.era() >= crate::Era::Town {
            self.strike_camp(now);
            return;
        }

        // --- 4. paths first ------------------------------------------------
        // The first cart track is worn out of the paths *between the camp and
        // the cabins* once the cabins stand — not out of the walk-in trail.
        if !self.settlers.road_laid && self.settlers.cabins >= 2 {
            if self.lay_first_track() {
                self.settlers.road_laid = true;
            }
        }

        // Stumps are pulled once the camp is established (B39 item 3).
        if !self.settlers.stumps_pulled && self.settlers.cabins >= 2 && self.settlers.felled > 5 {
            let before = self.settlers.piles.len();
            self.settlers.piles.retain(|p| p.kind == 0);
            if self.settlers.piles.len() != before {
                self.settlers.stumps_pulled = true;
                self.events.push(crate::Event {
                    time: now,
                    text: "the stumps are pulled and the ground levelled".to_string(),
                });
            }
        }
    }

    /// Wear the ground under every walking member for the leg covered since the
    /// last planning step (B40: the trail has to be continuous).
    fn wear_along_legs(&mut self, now: f64) {
        let prev = now - self.last_dt.max(0.5);
        let mut worn: Vec<(i32, i32)> = Vec::new();
        for m in &self.settlers.party {
            if m.kind == 1 {
                continue; // the wagon leaves ruts, not footpaths
            }
            let (ax, ay) = m.at(prev);
            let (bx, by) = m.at(now);
            let d = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            let n = (d * 3.0).ceil().max(1.0) as i32;
            for k in 0..=n {
                let f = k as f32 / n as f32;
                let x = (ax + (bx - ax) * f).floor() as i32;
                let y = (ay + (by - ay) * f).floor() as i32;
                if self.map.in_bounds(x, y) && !self.map.at(x, y).kind.is_lane() {
                    self.map.add_wear(x, y, 9);
                    worn.push((x, y));
                }
            }
        }
        for (x, y) in worn {
            self.note_worn(x, y);
            self.settlers.worn_trail.insert((x, y));
        }
    }

    /// A tree goes over: it is removed now and drawn leaning for a moment, then
    /// it becomes a stump and a log pile (B40: the fall has to be seen).
    fn fell_tree(&mut self, x: i32, y: i32, now: f64) {
        let t = self.map.at(x, y);
        let (size, variant) = match t.prop {
            map::Prop::Tree { size, variant, .. } => (size, variant),
            _ => (3, 0),
        };
        self.map.with_tile_mut(x, y, |t| t.prop = map::Prop::None);
        self.mark_changed(x, y);
        self.settlers.falling.push(Falling {
            x,
            y,
            t0: now,
            dur: FALL_SECONDS,
            dir: if (x + y) % 2 == 0 { 1 } else { -1 },
            size,
            variant,
        });
        self.settlers.felled += 1;
        self.events.push(crate::Event {
            time: now,
            text: format!("a tree comes down at {x},{y}"),
        });
    }

    /// The nearest unclaimed tree to a point, within `r` tiles.
    fn nearest_tree(
        &self,
        from: (f32, f32),
        r: i32,
        claimed: &[(i32, i32)],
    ) -> Option<(i32, i32)> {
        let (cx, cy) = (from.0.floor() as i32, from.1.floor() as i32);
        let mut best: Option<(i32, i32, i32)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (cx + dx, cy + dy);
                if !self.map.in_bounds(x, y) || !self.map.at(x, y).prop.is_tree() {
                    continue;
                }
                if claimed.contains(&(x, y)) {
                    continue;
                }
                let d = dx * dx + dy * dy;
                if best.as_ref().map(|(bd, _, _)| d < *bd).unwrap_or(true) {
                    best = Some((d, x, y));
                }
            }
        }
        best.map(|(_, x, y)| (x, y))
    }

    /// Stake the next log cabin next to the camp or an existing cabin.
    fn stake_cabin(&mut self) -> bool {
        let (fx, fy) = self.founding;
        for ring in 1..7i32 {
            for dy in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dy.abs() != ring {
                        continue;
                    }
                    let (x, y) = (fx + dx, fy + dy);
                    if !self.plot_is_free(x, y, 2, 1) {
                        continue;
                    }
                    // A cabin fronts the worn path, not a street (B39 item 4).
                    if !self.has_path_near(x, y, 2, 1) {
                        continue;
                    }
                    if self.place_planned(
                        crate::blocks::PlanLot {
                            x,
                            y,
                            w: 2,
                            h: 1,
                            role: crate::blocks::Role::Low,
                        },
                        LotKind::LogCabin,
                    ) {
                        self.events.push(crate::Event {
                            time: self.time,
                            text: format!(
                                "the {}th cabin is staked at {x},{y}",
                                self.settlers.cabins + 1
                            ),
                        });
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Pave the first cart track along the most-worn *contiguous* run of path
    /// between the camp and its cabins (B39 item 4, B40: no road at the camp).
    fn lay_first_track(&mut self) -> bool {
        let (fx, fy) = self.founding;
        let worn = |s: &Self, x: i32, y: i32| {
            map::WearStage::of(s.map.wear(x, y)) >= map::WearStage::Trampled
        };
        // Start from the most worn tile near the camp.
        let mut best: Option<(u8, (i32, i32))> = None;
        for dy in -4..=4 {
            for dx in -4..=4 {
                let (x, y) = (fx + dx, fy + dy);
                if !self.map.in_bounds(x, y) || !worn(self, x, y) {
                    continue;
                }
                let w = self.map.wear(x, y);
                if best.map(|(bw, _)| w > bw).unwrap_or(true) {
                    best = Some((w, (x, y)));
                }
            }
        }
        let Some((_, (bx, by))) = best else {
            return false;
        };
        let along_x = worn(self, bx - 1, by) || worn(self, bx + 1, by);
        let (dx, dy) = if along_x { (1, 0) } else { (0, 1) };
        let mut lane = vec![(bx, by)];
        for sgn in [1i32, -1] {
            let mut k = 1;
            while k < 6 {
                let (x, y) = (bx + dx * k * sgn, by + dy * k * sgn);
                if !worn(self, x, y) {
                    break;
                }
                lane.push((x, y));
                k += 1;
            }
        }
        if lane.len() < 2 {
            return false;
        }
        // Never pave through a cabin: the track follows the path *between* the
        // plots (B40 — the village test asserts no lot stands on a street).
        if self
            .lots
            .iter()
            .any(|l| lane.iter().any(|&(x, y)| l.covers(x, y)))
        {
            return false;
        }
        self.lay_lane(&lane, map::Kind::Dirt);
        self.stats.paths_paved += 1;
        self.events.push(crate::Event {
            time: self.time,
            text: "the path between the camp and the cabins is worn into a cart track".to_string(),
        });
        true
    }

    /// Is there a worn path (or lane) beside this plot?
    pub(crate) fn has_path_near(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        for ty in y - 1..=y + h {
            for tx in x - 1..=x + w {
                let t = self.map.at(tx, ty);
                if t.kind.is_lane() {
                    return true;
                }
                if map::WearStage::of(self.map.wear(tx, ty)) >= map::WearStage::Footpath {
                    return true;
                }
            }
        }
        false
    }

    /// Strike the camp (B41b): tents down, stumps pulled, log piles used up,
    /// the party gone. Called once; idempotent.
    pub(crate) fn strike_camp(&mut self, now: f64) {
        if self.settlers.struck {
            return;
        }
        let had_camp = self.settlers.camp.is_some() || !self.settlers.party.is_empty();
        self.settlers.camp = None;
        self.settlers.party.clear();
        self.settlers.piles.clear();
        self.settlers.falling.clear();
        self.settlers.struck = true;
        // A struck camp is a settled town: the planner (`decide`) waits for
        // `arrived`, and a loaded save strikes the camp of the fresh party
        // `Sim::new` spawned before it ever walked in. Without this a city
        // loaded from a save never built again (the owner: "the wallpaper is not
        // actually evolving over time").
        if !self.settlers.arrived {
            self.settlers.arrived = true;
            self.settlers.arrived_at = now;
        }
        // The worn trails the party left fade like any unused path.
        for (x, y) in std::mem::take(&mut self.settlers.worn_trail) {
            if self.map.in_bounds(x, y) && !self.map.at(x, y).kind.is_lane() {
                let w = self.map.wear(x, y).saturating_sub(80);
                self.map.wear[(y * self.map.w + x) as usize] = w;
                self.changes.insert((x, y));
            }
        }
        if had_camp {
            self.events.push(crate::Event {
                time: now,
                text: "the camp is struck: the tents come down and the last stumps go".to_string(),
            });
        }
    }

    /// Is this tile ground the settlement may still draw on? Nothing settler-
    /// related is drawn on a street, a building or a lot (B41b).
    fn settler_ground(&self, x: i32, y: i32) -> bool {
        if !self.map.in_bounds(x, y) {
            return false;
        }
        let t = self.map.at(x, y);
        if t.kind.is_lane() || t.kind.is_worked() || t.is_water() {
            return false;
        }
        !self.lots.iter().any(|l| l.covers(x, y))
    }

    /// The settlers' log piles and stumps, for the renderer — filtered to ground
    /// the village has not taken (B41b).
    pub fn log_piles(&self) -> Vec<LogPile> {
        self.settlers
            .piles
            .iter()
            .copied()
            .filter(|p| self.settler_ground(p.x, p.y))
            .collect()
    }

    /// Where the camp is, for the renderer: none once struck, and never on
    /// ground the village has taken.
    pub fn camp(&self) -> Option<Camp> {
        let c = self.settlers.camp.as_ref()?;
        let (fx, fy) = self.founding;
        let mut tents: Vec<(i32, i32)> = c
            .tents
            .iter()
            .copied()
            .filter(|&(dx, dy)| self.settler_ground(fx + dx, fy + dy))
            .collect();
        let fire = (fx + c.fire.0, fy + c.fire.1);
        if tents.is_empty() && !self.settler_ground(fire.0, fire.1) {
            return None;
        }
        if !self.settler_ground(fire.0, fire.1) {
            // The fire's ground has been paved: keep the tents only.
            tents.retain(|_| true);
            return Some(Camp {
                tents,
                fire: c.fire,
                lit: c.lit,
                fire_ok: false,
            });
        }
        Some(Camp {
            tents,
            fire: c.fire,
            lit: c.lit,
            fire_ok: true,
        })
    }

    /// The party at a smooth clock time: (x, y, kind, phase) where `phase` is
    /// 0..1 through the chop swing (or the walk cycle) for the renderer.
    pub fn settlers_at(&self, t: f64) -> Vec<(f32, f32, u8, f32)> {
        self.settlers
            .party
            .iter()
            .filter(|m| {
                let (x, y) = m.at(t);
                self.settler_ground(x.floor() as i32, y.floor() as i32)
            })
            .map(|m| {
                let (x, y) = m.at(t);
                let phase = if m.state == 1 {
                    ((m.busy_until - t) * 2.0).rem_euclid(1.0) as f32
                } else {
                    (t as f32 * 1.6 + m.t0 as f32).rem_euclid(1.0)
                };
                (x, y, m.kind, phase)
            })
            .collect()
    }

    /// The trees going over right now: (x, y, fraction fallen, dir, size, variant).
    pub fn falling_at(&self, t: f64) -> Vec<(i32, i32, f32, i8, u8, u8)> {
        self.settlers
            .falling
            .iter()
            .filter(|f| self.settler_ground(f.x, f.y))
            .map(|f| {
                let frac = (((t - f.t0) / f.dur) as f32).clamp(0.0, 1.0);
                (f.x, f.y, frac, f.dir, f.size, f.variant)
            })
            .collect()
    }
}

/// A log cabin is the settlement's house: logs, then walls, then roof (B39).
pub fn cabin_stages() -> [Stage; 3] {
    [Stage::Framing, Stage::Scaffold, Stage::Finished]
}

/// The trees the camp still has to cut: used by the tests.
pub fn trees_near(s: &Sim, r: i32) -> usize {
    let (fx, fy) = s.founding;
    let mut n = 0;
    for dy in -r..=r {
        for dx in -r..=r {
            if s.map.at(fx + dx, fy + dy).prop.is_tree() {
                n += 1;
            }
        }
    }
    n
}
