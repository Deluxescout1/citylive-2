//! B77/D84: the city's weekly rhythm — the week you can see.
//!
//! A Tuesday, a Saturday and a Sunday must not be the same picture. This module
//! is the calendar's share of that: bin day per district, the Saturday market,
//! the school run and its holidays, the Sunday morning, the small hours (the
//! bakery, the night bus, the sweeper, the taxis) and the extra traffic a
//! Saturday brings. The residents' own day is `people::schedule_at`; this is
//! what the *town* does on top of it.
//!
//! Like `streetlife`, **nothing is stored**. Every bin, stall, jogger and van is
//! a pure function of the sim clock, the day of the week, the seed and what the
//! city has built, so a reload shows the same Tuesday and a catch-up cannot
//! drift from a live run. The actors ride the two layers that already exist —
//! the people layer (an object figure is a prop with no body) and the traffic
//! layer (a `Vehicle` on a short street run) — so there is no new per-frame
//! system, only more actors (D71).

use crate::landmarks::LandmarkKind;
use crate::people::{door_tile, AgeGroup, Figure, Pose, Prop, TONES};
use crate::traffic::{Vehicle, VehicleKind};
use crate::{District, Lot, LotKind, Sim};
use map::Kind as TileKind;

// --- the week ---------------------------------------------------------------

pub const SUNDAY: i32 = 0;
pub const MONDAY: i32 = 1;
pub const TUESDAY: i32 = 2;
pub const WEDNESDAY: i32 = 3;
pub const THURSDAY: i32 = 4;
pub const FRIDAY: i32 = 5;
pub const SATURDAY: i32 = 6;

/// The day of the market.
pub const MARKET_DAY: i32 = SATURDAY;

/// The morning the bins go out, one weekday per district: the round works the
/// town from its industrial edge on Monday to the high end on Friday, so no
/// street ever sees two lorries in one day (D84).
pub fn bin_day(d: District) -> i32 {
    match d {
        District::Industrial => MONDAY,
        District::Core => TUESDAY,
        District::European => WEDNESDAY,
        District::Suburb => THURSDAY,
        District::Downtown | District::HighEnd => FRIDAY,
    }
}

/// The weekday's name, for a story line or a debug print.
pub fn day_name(dow: i32) -> &'static str {
    match dow.rem_euclid(7) {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        _ => "Saturday",
    }
}

impl Sim {
    /// Today's day of the week: 0 = Sunday (the same count the people use).
    pub fn week_day(&self) -> i32 {
        self.weekday() as i32
    }

    /// B77: has the town a calendar date? The schools need one for their terms;
    /// with no date the city keeps a term-time week, which is the honest
    /// default for a world that is not tied to a real month.
    pub fn set_calendar_date(&mut self, year: i32, month: u8, day: u8) {
        self.calendar = Some((year, month, day));
    }

    /// Is the school in session today? Weekends and the school holidays are out
    /// (D84: an empty playground in August, a full one in October).
    pub fn school_in_session(&self) -> bool {
        let dow = self.week_day();
        if dow == SUNDAY || dow == SATURDAY {
            return false;
        }
        match self.calendar {
            Some((y, m, d)) => !school_holiday(y, m, d),
            None => true,
        }
    }

    /// The lot the town keeps its school in (D82's school landmark).
    pub fn school_lot(&self) -> Option<&Lot> {
        self.lots.iter().find(|l| {
            l.finished() && matches!(l.kind, LotKind::Landmark(LandmarkKind::School))
        })
    }

    /// The shop the town treats as its bakery: the store nearest the square (a
    /// bakery is a shop with an oven behind it, and the town has one). `None`
    /// before there is a store at all.
    pub fn bakery_index(&self) -> Option<usize> {
        let square = self
            .lots
            .iter()
            .find(|l| l.finished() && matches!(l.kind, LotKind::Landmark(LandmarkKind::Plaza)))
            .map(|l| (l.x + l.w / 2, l.y + l.h / 2));
        let mut best: Option<(i64, usize)> = None;
        for (i, l) in self.lots.iter().enumerate() {
            if !l.finished() || !matches!(l.kind, LotKind::Store | LotKind::Shop) {
                continue;
            }
            let (cx, cy) = (l.x + l.w / 2, l.y + l.h / 2);
            let d = match square {
                Some((sx, sy)) => ((cx - sx) as i64).pow(2) + ((cy - sy) as i64).pow(2),
                None => (i as i64) * 1_000_000,
            };
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        best.map(|b| b.1)
    }

    /// The square the market goes up on: the civic plaza if the town has one,
    /// else a car park, else the largest green (D84 says a square or a car park;
    /// an early town has neither and uses its green).
    pub fn market_place(&self) -> Option<&Lot> {
        let plaza = self
            .lots
            .iter()
            .find(|l| l.finished() && matches!(l.kind, LotKind::Landmark(LandmarkKind::Plaza)));
        if plaza.is_some() {
            return plaza;
        }
        let mut parks: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| l.finished() && l.kind == LotKind::CarPark && l.w >= 4 && l.h >= 3)
            .collect();
        parks.sort_by_key(|l| std::cmp::Reverse(l.w * l.h));
        if let Some(l) = parks.first() {
            return Some(l);
        }
        let mut greens: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| l.finished() && l.kind == LotKind::Park && l.w >= 4 && l.h >= 4)
            .collect();
        greens.sort_by_key(|l| std::cmp::Reverse(l.w * l.h));
        greens.first().copied()
    }

    /// Is the market on today, at this hour? Saturday only — stalls go up from
    /// six, trade through the day, and are packed away by half past six.
    pub fn market_hours(&self, hour: f64) -> bool {
        self.week_day() == MARKET_DAY && (6.0..18.5).contains(&hour)
    }

    /// How many stalls are standing at `hour` (0 outside the market, a growing
    /// number as the vans unload, a shrinking one as they pack up).
    pub fn market_stalls(&self, hour: f64) -> usize {
        if !self.market_hours(hour) {
            return 0;
        }
        let Some(l) = self.market_place() else {
            return 0;
        };
        let all = ((l.w.min(l.h) as usize) * 2).clamp(4, 12);
        if hour < 7.4 {
            // Going up: one stall every ten minutes or so.
            (((hour - 6.0) / 1.4) * all as f64).clamp(0.0, all as f64) as usize
        } else if hour < 16.6 {
            all
        } else {
            // Packed up: the stallholders load the last of it by half past six.
            let left = ((18.2 - hour) / 0.9 * all as f64).clamp(0.0, all as f64);
            left as usize
        }
    }

    // --- bins ---------------------------------------------------------------

    /// Is a bin collection due in this district today, and still to come?
    fn bin_today(&self, l: &Lot, hour: f64) -> bool {
        let d = self.district_at(l.x + l.w / 2, l.y + l.h / 2);
        let bd = bin_day(d);
        let today = self.week_day();
        // Out from the evening before, back in once the lorry has been.
        let evening_before = bd == (today + 1) % 7 && hour >= 19.0;
        let morning = bd == today && hour < 15.0;
        evening_before || morning
    }

    /// B77: the bins at the kerb. One or two per household on its district's
    /// bin day — out the evening before, taken back in during the day.
    pub fn bins_out_into(&self, hour: f64, out: &mut Vec<Figure>) {
        for l in self.lots.iter() {
            if !l.finished() {
                continue;
            }
            // Flats and towers have a bin store, not a row of bins at the kerb.
            if !matches!(l.kind, LotKind::House | LotKind::Townhouse) {
                continue;
            }
            if !self.bin_today(l, hour) {
                continue;
            }
            let (x, y, dir) = self.kerb_spot(l);
            let n = if l.seed % 4 == 0 { 2 } else { 1 };
            let (lx, ly) = lateral(l.front);
            for k in 0..n {
                out.push(Figure {
                    x: x + lx * (0.45 * k as f32),
                    y: y + ly * (0.45 * k as f32),
                    pose: Pose::Object,
                    // The bin's own colour: green, black or blue, so a row of
                    // them down a street is not one stamp repeated.
                    tone: ((l.seed as usize + k * 5) % 3) as u8,
                    child: false,
                    prop: Prop::Bin,
                    dir,
                    open: false,
                    lift: 0,
                });
            }
        }
    }

    /// A tile at the kerb of a plot's frontage, where its bins stand: on the
    /// pavement, half a tile out toward the road, never in the carriageway.
    fn kerb_spot(&self, l: &Lot) -> (f32, f32, u8) {
        let (dx, dy) = door_tile(l);
        let (ox, oy) = outward(l.front);
        let (lx, ly) = lateral(l.front);
        // Slide along the frontage until a tile that is not a lane turns up.
        for s in [0.0f32, -0.6, 0.6, -1.2, 1.2, -1.8, 1.8, -2.4, 2.4] {
            let x = dx as f32 + lx * s + ox * 0.15;
            let y = dy as f32 + ly * s + oy * 0.15;
            let kind = self.map.at(x.floor() as i32, y.floor() as i32).kind;
            if kind.is_lane() {
                continue;
            }
            if kind == TileKind::Sidewalk || kind.is_walkable() {
                return (x, y, l.front);
            }
        }
        // No pavement on this street: the bins stand just inside the frontage,
        // on the household's own ground, rather than in the carriageway.
        (
            dx as f32 + 0.5 - ox * 0.85,
            dy as f32 + 0.5 - oy * 0.85,
            l.front,
        )
    }

    // --- the school ---------------------------------------------------------

    /// B77: the school run — children walking to school with a parent, a
    /// lollipop warden on the crossing, the bus at the gate. Nothing on a
    /// weekend and nothing in the holidays.
    pub fn school_run_into(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        if !self.school_in_session() {
            return;
        }
        let Some(school) = self.school_lot() else {
            return;
        };
        let (sx, sy) = door_tile(school);
        // Two windows: the morning run and the afternoon pick-up.
        let (start, end, to_work) = if (7.35..9.1).contains(&hour) {
            (7.35, 9.1, true)
        } else if (14.85..16.2).contains(&hour) {
            (14.85, 16.2, false)
        } else {
            return;
        };
        let f = ((hour - start) / (end - start)).clamp(0.0, 1.0) as f32;

        // The warden at the crossing: hi-vis on the kerb, one arm up.
        if (7.4..9.2).contains(&hour) || (14.8..16.3).contains(&hour) {
            out.push(Figure {
                x: sx as f32 + 0.5,
                y: sy as f32 + 1.1,
                pose: Pose::Stand,
                tone: 8,
                child: false,
                prop: Prop::None,
                dir: 1,
                open: false,
                lift: 0,
            });
        }

        // The children come along the school's own frontages — the two faces
        // the camera sees — from both far corners to the gate: a school run is
        // a knot of children at the school, and the school's plot is where they
        // are guaranteed to be beside it (the nearest street can be ten tiles
        // away behind a big plot).
        // Two tiles clear of the wall: the kit's roofs overhang their plot, and
        // a child hugging the wall is a child behind the eaves.
        let (gx, gy) = (school.x as f32 + school.w as f32 + 1.7, school.y as f32 + school.h as f32 + 1.7);
        let leg_from = [
            (school.x as f32 + school.w as f32 + 1.7, school.y as f32 - 0.8),
            (school.x as f32 - 0.8, school.y as f32 + school.h as f32 + 1.7),
        ];
        let mut kids: Vec<u8> = Vec::new();
        for p in self.people.list.iter() {
            if !p.alive() || p.kind != AgeGroup::Child {
                continue;
            }
            kids.push(p.tone);
            if kids.len() >= 56 {
                break;
            }
        }
        let count = kids.len().max(1);
        for (i, tone) in kids.iter().enumerate() {
            // Half the stream comes down each frontage; the clock walks them
            // from their corner to the gate over the window.
            let from = leg_from[i % 2];
            // A school run reads as a knot of children *at the gate*, thinning
            // out along the wall.
            let rank = i as f32 / count as f32;
            let spread = 0.10 + 0.55 * rank;
            let q = if to_work {
                (1.0 - spread + f * 0.12).clamp(0.0, 1.0)
            } else {
                (spread - f * 0.12).clamp(0.0, 1.0)
            };
            let x = from.0 + (gx - from.0) * q;
            let y = from.1 + (gy - from.1) * q;
            let dir = dir_of(gx - from.0, gy - from.1);
            let bob = ((time as f32 * 6.0 + i as f32).sin() * 0.06) as f32;
            out.push(Figure {
                x,
                y: y + bob,
                pose: Pose::Walk,
                tone: *tone,
                child: true,
                prop: if i % 9 == 0 { Prop::Bag } else { Prop::None },
                dir,
                open: true,
                lift: 0,
            });
            // Every third child is walked in by a parent, half a tile behind.
            if i % 3 == 0 {
                out.push(Figure {
                    x: x - (gx - from.0) * 0.02,
                    y: y - (gy - from.1) * 0.02,
                    pose: Pose::Walk,
                    tone: ((i * 3 + 1) % 8) as u8,
                    child: false,
                    prop: Prop::None,
                    dir,
                    open: true,
                    lift: 0,
                });
            }
        }
        // A knot of children at the gate in the morning, waiting to go in: on
        // the street by the school, where the run reaches it.
        // (A school still standing after its children have gone has no knot at the
        // gate: `kids` is empty, and the pick below would divide by zero.)
        if to_work && !kids.is_empty() {
            for k in 0..14usize {
                let a = -1.25 + 2.5 * k as f32 / 13.0;
                out.push(Figure {
                    x: gx - 0.55 + a * 0.55,
                    y: gy - 0.55 + a.abs() * 0.3,
                    pose: if k % 3 == 0 { Pose::Chat } else { Pose::Stand },
                    tone: kids[k % kids.len()],
                    child: true,
                    prop: Prop::None,
                    dir: school.front,
                    open: false,
                    lift: 0,
                });
            }
        }
    }

    /// B77: the playground at break — a yard full of children twice a day, and
    /// nobody at all in the holidays.
    pub fn playground_into(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        if !self.school_in_session() {
            return;
        }
        let Some(school) = self.school_lot() else {
            return;
        };
        let brk = (10.0..10.35).contains(&hour) || (12.25..12.7).contains(&hour) || (14.9..15.1).contains(&hour);
        if !brk {
            return;
        }
        let n = 10 + (school.seed % 5) as usize;
        for k in 0..n {
            let a = 0.6 + 6.283 * k as f32 / n as f32;
            let r = 0.28 + ((k % 3) as f32) * 0.11;
            let cx = school.x as f32 + school.w as f32 * 0.5 + a.cos() * r * school.w as f32 * 0.5;
            let cy = school.y as f32 + school.h as f32 * 0.5 + a.sin() * r * school.h as f32 * 0.45;
            let run = ((time as f32 * 3.0 + k as f32 * 1.7).sin() * 0.3) as f32;
            out.push(Figure {
                x: cx + run,
                y: cy,
                pose: if k % 3 == 0 { Pose::Play } else { Pose::Walk },
                tone: ((k * 5 + school.seed as usize) % 8) as u8,
                child: true,
                prop: if k % 4 == 1 { Prop::Ball } else { Prop::None },
                dir: (k % 4) as u8,
                open: true,
                lift: 0,
            });
        }
    }

    // --- the market ---------------------------------------------------------

    /// B77: the Saturday market — a row of stalls on the square, each with its
    /// stallholder behind the counter, crates of produce and shoppers between
    /// them.
    pub fn market_figures_into(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        let n = self.market_stalls(hour);
        if n == 0 {
            return;
        }
        let Some(l) = self.market_place() else {
            return;
        };
        let along_x = l.w >= l.h;
        let (ox, oy) = if along_x { (0.0f32, 1.0f32) } else { (1.0, 0.0) };
        let span = if along_x { l.w } else { l.h };
        // Two facing rows down the square's two longest edges, with the aisle
        // between them and the fountain in the middle: the way a market fills a
        // square, and a layout that keeps the stalls out of the water.
        for k in 0..n {
            let t = ((k / 2) as f32 + 0.5) / ((n + 1) / 2).max(1) as f32;
            let u = 0.5 + (t - 0.5) * 0.88;
            let across = if k % 2 == 0 { 0.26 } else { 0.74 };
            let (x, y) = if along_x {
                (l.x as f32 + l.w as f32 * u, l.y as f32 + l.h as f32 * across)
            } else {
                (l.x as f32 + l.w as f32 * across, l.y as f32 + l.h as f32 * u)
            };
            let dir = if along_x { 0 } else { 2 };
            // The market goes round the fountain, not through it.
            if self.in_open_water(l, x, y) {
                continue;
            }
            // The stall itself.
            out.push(Figure {
                x,
                y,
                pose: Pose::Object,
                // The awning colour alternates along each row, not between them.
                tone: ((k / 2) % 2) as u8,
                child: false,
                prop: Prop::Stall,
                dir,
                open: true,
                lift: 0,
            });
            // Its stallholder behind the counter, and the crates at the side.
            let back = if k % 2 == 0 { 0.45 } else { -0.45 };
            let bob = ((time as f32 * 1.7 + k as f32).sin() * 0.08) as f32;
            out.push(Figure {
                x: x + ox * back,
                y: y + oy * back + bob,
                pose: if k % 3 == 0 { Pose::Wave } else { Pose::Stand },
                tone: ((k * 5 + 2) % TONES) as u8,
                child: false,
                prop: Prop::None,
                dir: if along_x { 1 } else { 3 },
                open: true,
                lift: 0,
            });
            if k % 3 == 1 {
                out.push(Figure {
                    x: x - ox * 0.7 + if along_x { 0.8 } else { 0.0 },
                    y: y - oy * 0.7 + if along_x { 0.0 } else { 0.8 },
                    pose: Pose::Object,
                    tone: 3,
                    child: false,
                    prop: Prop::Box,
                    dir,
                    open: true,
                    lift: 0,
                });
            }
            // Shoppers between the stalls: a slow drift, so the crowd moves.
            let shoppers = if hour > 7.6 && hour < 16.4 { 7 } else { 1 };
            for m in 0..shoppers {
                let h = hash3(k as u32, m as u32, l.seed);
                let su = (t + (h % 100) as f32 / 600.0) % 1.0;
                // In the aisle between the rows, and up against the stalls.
                let across = if m % 2 == 0 { 0.40 } else { 0.60 }
                    + ((h >> 8) % 40) as f32 / 100.0 * 0.1;
                let (pqx, pqy) = if along_x {
                    (l.x as f32 + l.w as f32 * (0.5 + (su - 0.5) * 0.9), l.y as f32 + l.h as f32 * across)
                } else {
                    (l.x as f32 + l.w as f32 * across, l.y as f32 + l.h as f32 * (0.5 + (su - 0.5) * 0.9))
                };
                let drift = ((time as f32 * 0.32 + m as f32 * 2.1 + k as f32).sin() * 0.5) as f32;
                if self.in_open_water(l, pqx, pqy) {
                    continue;
                }
                out.push(Figure {
                    x: pqx + if along_x { drift } else { 0.0 },
                    y: pqy + if along_x { 0.0 } else { drift },
                    pose: if m % 3 == 0 { Pose::Chat } else { Pose::Walk },
                    tone: ((h >> 16) as usize % TONES) as u8,
                    child: m == 2 && h % 5 == 0,
                    prop: if h % 4 == 0 { Prop::Bag } else { Prop::None },
                    dir: ((h >> 4) % 4) as u8,
                    open: true,
                    lift: 0,
                });
            }
        }
        let _ = span;
    }

    // --- Sunday morning and the small hours ---------------------------------

    /// B77: a Sunday morning — joggers and dog walkers early, the church crowd
    /// at ten, the parks filling in the afternoon.
    pub fn sunday_life_into(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        if self.week_day() != SUNDAY {
            return;
        }
        let parks: Vec<&Lot> = self
            .lots
            .iter()
            .filter(|l| {
                l.finished()
                    && (l.kind == LotKind::Park
                        || matches!(l.kind, LotKind::Landmark(LandmarkKind::Plaza)))
            })
            .collect();
        // Joggers and dog walkers, first light: round the park, on the grass.
        if (6.2..9.2).contains(&hour) && !parks.is_empty() {
            let jog = 6usize;
            for k in 0..jog {
                let p = parks[k % parks.len()];
                let a = 1.1 + 6.283 * (k as f32 / jog as f32);
                let r = 0.34;
                let pace = time as f32 * 0.55 + k as f32;
                let (x, y) = (
                    p.x as f32 + p.w as f32 * (0.5 + a.cos() * r * (0.9 + 0.06 * pace.sin())),
                    p.y as f32 + p.h as f32 * (0.5 + a.sin() * r * 0.8),
                );
                out.push(Figure {
                    x,
                    y,
                    pose: Pose::Walk,
                    tone: ((k * 3 + 1) % 8) as u8,
                    child: false,
                    prop: if k % 2 == 0 { Prop::Dog } else { Prop::None },
                    dir: dir_of(-a.sin(), a.cos()),
                    open: true,
                    lift: 0,
                });
            }
        }
        // The church crowd, ten o'clock: a stream up the path, then the steps.
        // The town's place of worship is its church if it has one, else the
        // cathedral (D82), else the town hall.
        let church = self
            .lots
            .iter()
            .find(|l| l.finished() && l.kind == LotKind::Church)
            .or_else(|| {
                self.lots.iter().find(|l| {
                    l.finished()
                        && matches!(l.kind, LotKind::Landmark(LandmarkKind::Cathedral))
                })
            })
            .or_else(|| {
                self.lots
                    .iter()
                    .find(|l| l.finished() && l.kind == LotKind::Hall)
            });
        if let Some(c) = church {
            let (cx, cy) = door_tile(c);
            if (9.3..10.3).contains(&hour) {
                // Walking up the street to the door, from both ends of the run
                // the church stands on — a Sunday crowd is a crowd at the
                // church, not a scatter of people crossing back gardens.
                if let Some(run) = self.street_run_near((cx, cy), 10, 0xC4) {
                    let n = run.len();
                    let gate = (0..n)
                        .min_by_key(|&i| (run[i].0 - cx).abs() + (run[i].1 - cy).abs())
                        .unwrap_or(n / 2);
                    let f = ((hour - 9.3) / 1.0).clamp(0.0, 1.0) as f32;
                    for k in 0..16usize {
                        let h = hash3(c.seed, k as u32, 7);
                        let from = if k % 2 == 0 { 0 } else { n - 1 };
                        // The early ones are already inside; the rest trail in.
                        let q = (f * 1.5 - (k as f32 / 16.0) * 0.55).clamp(0.0, 1.0);
                        let pos = from as f32 + (gate as f32 - from as f32) * q;
                        let idx = (pos.floor() as usize).min(n.saturating_sub(1));
                        let frac = (pos - idx as f32).clamp(0.0, 1.0);
                        let (ax, ay) = (run[idx].0 as f32 + 0.5, run[idx].1 as f32 + 0.5);
                        let (bx, by) = match run.get(idx + 1) {
                            Some(b) => (b.0 as f32 + 0.5, b.1 as f32 + 0.5),
                            None => (ax, ay),
                        };
                        let (dx, dy) = (bx - ax, by - ay);
                        let len = (dx * dx + dy * dy).sqrt().max(0.001);
                        let side = 0.38 * if k % 2 == 0 { 1.0 } else { -1.0 };
                        out.push(Figure {
                            x: ax + dx * frac + (-dy / len) * side,
                            y: ay + dy * frac + (dx / len) * side,
                            pose: Pose::Walk,
                            // Sunday best: the muted end of the palette.
                            tone: ((h >> 4) as usize % 6) as u8,
                            child: k % 6 == 0,
                            prop: Prop::None,
                            dir: dir_of(dx * (gate as f32 - pos), dy * (gate as f32 - pos)),
                            open: false,
                            lift: 0,
                        });
                    }
                }
            } else if (10.2..11.4).contains(&hour) {
                // On the steps, and a few arriving late.
                for k in 0..9usize {
                    let a = -1.2 + 2.4 * k as f32 / 8.0;
                    out.push(Figure {
                        x: cx as f32 + 0.5 + a * 0.7,
                        y: cy as f32 + 0.9 + (a.abs()) * 0.3,
                        pose: if k % 4 == 0 { Pose::Chat } else { Pose::Stand },
                        tone: ((k * 2 + 1) % 6) as u8,
                        child: k == 3,
                        prop: Prop::None,
                        dir: c.front,
                        open: false,
                        lift: 0,
                    });
                }
            }
        }
        // The parks in the afternoon: busier than any weekday.
        if (13.0..17.5).contains(&hour) {
            for (pi, p) in parks.iter().enumerate() {
                let n = 7usize;
                for k in 0..n {
                    let h = hash3(p.seed, k as u32, pi as u32);
                    let x = p.x as f32 + 0.8 + ((h % 1000) as f32 / 1000.0) * (p.w as f32 - 1.6);
                    let y = p.y as f32 + 0.8 + (((h >> 10) % 1000) as f32 / 1000.0) * (p.h as f32 - 1.6);
                    out.push(Figure {
                        x,
                        y,
                        pose: if k % 3 == 0 { Pose::Sit } else { Pose::Chat },
                        tone: ((h >> 3) as usize % TONES) as u8,
                        child: k % 4 == 0,
                        prop: if k % 5 == 0 { Prop::Ball } else { Prop::None },
                        dir: (k % 4) as u8,
                        open: true,
                        lift: 0,
                    });
                }
            }
        }
    }

    /// B77: the small hours — the bakery's van at the kerb before five, a
    /// newspaper van at dawn, the night bus and the sweeper at three, the taxis
    /// after the bars close.
    pub fn small_hours_figures_into(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        if !(0.0..6.0).contains(&hour) {
            return;
        }
        // The baker at the door with the first trays, from half past four.
        if let Some(bk) = self.bakery_index().and_then(|i| self.lots.get(i)) {
            if (4.3..5.6).contains(&hour) {
                let (dx, dy) = door_tile(bk);
                let tray = ((time as f32 * 0.5).sin() * 0.06) as f32;
                out.push(Figure {
                    x: dx as f32 + 0.4,
                    y: dy as f32 + 0.3 + tray,
                    pose: Pose::Carry,
                    tone: 2,
                    child: false,
                    prop: Prop::Box,
                    dir: bk.front,
                    open: false,
                    lift: 0,
                });
            }
        }
    }

    // --- the week's vehicles ------------------------------------------------

    /// B77: every vehicle the week puts on the road that the traffic sim does
    /// not generate itself — the bin lorry on its round, the market's vans, the
    /// school bus, the bakery's delivery, the night bus, the sweeper, the taxis.
    /// Each is a `Vehicle` on a short run of one street, so it is posed and
    /// drawn by the traffic layer exactly like a car.
    pub fn weekly_vehicles(&self) -> Vec<Vehicle> {
        let mut out = Vec::new();
        self.weekly_vehicles_into(&mut out);
        out
    }

    /// The same into a caller-owned buffer (the wallpaper calls this at the
    /// display rate, so it must not allocate once the buffer is warm).
    pub fn weekly_vehicles_into(&self, out: &mut Vec<Vehicle>) {
        let hour = (self.time / 3600.0).rem_euclid(24.0);
        let today = self.week_day();

        // 1. The bin lorry: one district a day, 06:45 to half past one.
        if (6.75..13.5).contains(&hour) {
            for (k, d) in bin_districts(today).iter().enumerate() {
                if let Some(run) = self.district_run(*d, 0x814 + k as u64) {
                    let span = run_len(&run);
                    let travelled = pingpong(self.time + k as f64 * 700.0, span, 0.55, 2.6);
                    out.push(sched_vehicle(
                        VehicleKind::Truck,
                        crate::traffic::style::BIN_LORRY,
                        run,
                        travelled,
                        self.seed ^ (0xB1 + k as u64),
                    ));
                }
            }
        }

        // 2. The market's vans: at the square before the stalls go up, and
        //    again while they come down.
        let market_hour = hour;
        if self.market_hours(market_hour) {
            let going_up = market_hour < 7.3;
            let packing = market_hour > 16.5;
            if going_up || packing {
                if let Some(l) = self.market_place() {
                    let (dx, dy) = door_tile(l);
                    for k in 0..3u32 {
                        let lane = self.nearest_lane((dx + k as i32, dy)).unwrap_or((dx, dy));
                        let mut route = vec![lane];
                        route.push(lane);
                        out.push(sched_vehicle(
                            VehicleKind::Van,
                            crate::traffic::style::DELIVERY_VAN,
                            route,
                            0.0,
                            self.seed ^ (0x4D00 + k as u64),
                        ));
                    }
                }
            }
        }

        // 3. The school bus: at the gate ten past eight, and again at twenty
        //    past three, in term time only.
        if self.school_in_session() {
            if let Some(school) = self.school_lot() {
                let (sx, sy) = door_tile(school);
                let gate = (7.95..8.4).contains(&hour) || (15.15..15.7).contains(&hour);
                if gate {
                    if let Some(run) = self.street_run_near((sx, sy), 10, 0x5C4001) {
                        let span = run_len(&run);
                        out.push(sched_vehicle(
                            VehicleKind::Bus,
                            crate::traffic::style::SCHOOL_BUS,
                            run,
                            span * 0.45,
                            self.seed ^ 0x5C40,
                        ));
                    }
                }
            }
        }

        // 4. The bakery's delivery before dawn, and the newspaper van at dawn.
        if let Some(bk) = self.bakery_index().and_then(|i| self.lots.get(i)) {
            let (dx, dy) = door_tile(bk);
            if (4.2..5.4).contains(&hour) {
                if let Some(lane) = self.nearest_lane((dx, dy)) {
                    out.push(sched_vehicle(
                        VehicleKind::Van,
                        crate::traffic::style::DELIVERY_VAN,
                        vec![lane, lane],
                        0.0,
                        self.seed ^ 0xBA4E,
                    ));
                }
            }
            if (4.6..6.2).contains(&hour) {
                if let Some(run) = self.street_run_near((dx, dy), 12, 0x9E45) {
                    let span = run_len(&run);
                    out.push(sched_vehicle(
                        VehicleKind::Van,
                        crate::traffic::style::DELIVERY_VAN,
                        run,
                        pingpong(self.time, span, 0.7, 3.5),
                        self.seed ^ 0x9E45,
                    ));
                }
            }
        }

        // 5. The small hours: a night bus, the sweeper, two taxis home from the
        //    bars.
        if (0.6..4.4).contains(&hour) {
            if let Some(run) = self.district_run(District::Core, 0xB05) {
                let span = run_len(&run);
                out.push(sched_vehicle(
                    VehicleKind::Bus,
                    crate::traffic::style::COACH,
                    run.clone(),
                    pingpong(self.time, span, 1.1, 6.0),
                    self.seed ^ 0xB05,
                ));
            }
        }
        if (2.9..5.2).contains(&hour) {
            if let Some(run) = self.district_run(District::Core, 0x5EE) {
                let span = run_len(&run);
                out.push(sched_vehicle(
                    VehicleKind::Van,
                    crate::traffic::style::SWEEPER,
                    run,
                    pingpong(self.time + 90.0, span, 0.45, 1.2),
                    self.seed ^ 0x5EE,
                ));
            }
        }
        if (1.1..3.4).contains(&hour) {
            if let Some(l) = self.market_place() {
                let (dx, dy) = door_tile(l);
                for k in 0..2u32 {
                    if let Some(run) = self.street_run_near((dx + k as i32 * 2, dy), 6, 0x7A81 + k as u64) {
                        let span = run_len(&run);
                        out.push(sched_vehicle(
                            VehicleKind::Taxi,
                            crate::traffic::style::TAXI,
                            run,
                            pingpong(self.time + 40.0 * k as f64, span, 0.9, 4.0),
                            self.seed ^ (0x7A81 + k as u64),
                        ));
                    }
                }
            }
        }
    }

    /// B77: everything the weekly rhythm adds, into the per-frame figure buffer.
    pub fn weekly_figures(&self, hour: f64, time: f64, out: &mut Vec<Figure>) {
        if std::env::var("B77_OFF").is_ok() {
            return;
        }
        let first = out.len();
        self.bins_out_into(hour, out);
        // The bin lorry's crew: two hands walking the kerb behind it, hi-vis
        // on, wheeling the bins out to the truck (D84: the round you can watch).
        if (6.75..13.5).contains(&hour) {
            let mut vans = Vec::new();
            self.weekly_vehicles_into(&mut vans);
            for v in vans.iter() {
                if v.style != crate::traffic::style::BIN_LORRY {
                    continue;
                }
                let ((x, y), d) = v.pose();
                let (lx, ly) = (-d.1, d.0);
                for k in 0..2 {
                    let off = 0.55 - k as f32 * 1.1;
                    out.push(Figure {
                        x: x - d.0 * 1.1 + lx * off,
                        y: y - d.1 * 1.1 + ly * off,
                        pose: Pose::Walk,
                        tone: 8,
                        child: false,
                        prop: if k == 0 { Prop::Bin } else { Prop::None },
                        dir: dir_of(-d.0, -d.1),
                        open: false,
                        lift: 0,
                    });
                }
            }
        }
        self.market_figures_into(hour, time, out);
        self.school_run_into(hour, time, out);
        self.playground_into(hour, time, out);
        self.sunday_life_into(hour, time, out);
        self.small_hours_figures_into(hour, time, out);
        // B77: the pavement rule, the same one the residents keep (B75): a
        // figure standing, sitting or chatting belongs on the pavement, never in
        // the carriageway — a market stall, a bin, a gate queue and a warden all
        // land on a lane if nobody says otherwise.
        for f in out[first..].iter_mut() {
            if f.pose == Pose::Walk {
                continue;
            }
            if self.map.at(f.x.floor() as i32, f.y.floor() as i32).kind.is_lane() {
                if let Some((x, y)) = self.nearest_pave(f.x, f.y) {
                    f.x = x;
                    f.y = y;
                }
            }
        }
    }

    /// The nearest tile to `(x, y)` that a standing figure can keep to: a
    /// pavement or any open ground that is not a lane.
    fn nearest_pave(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        for r in 1..4 {
            for (dx, dy) in [(0, 1), (1, 0), (0, -1), (-1, 0), (1, 1), (-1, 1), (1, -1), (-1, -1)] {
                let px = x + dx as f32 * r as f32 * 0.5;
                let py = y + dy as f32 * r as f32 * 0.5;
                let t = self.map.at(px.floor() as i32, py.floor() as i32).kind;
                if self.map.in_bounds(px.floor() as i32, py.floor() as i32)
                    && t.is_walkable()
                    && !t.is_lane()
                {
                    return Some((px, py));
                }
            }
        }
        None
    }

    /// The week's figures as a list (the tests, and any caller with no buffer
    /// of its own).
    pub fn weekly_figure_list(&self, hour: f64, time: f64) -> Vec<Figure> {
        let mut out = Vec::new();
        self.weekly_figures(hour, time, &mut out);
        out
    }

    // --- geometry helpers ---------------------------------------------------

    /// A run of one street in a district: the longest straight stretch of lane
    /// near the district's own lots. Cheap (a local scan, no A*), and the same
    /// every time, so the lorry works the same street all morning.
    fn district_run(&self, want: District, salt: u64) -> Option<Vec<(i32, i32)>> {
        let mut seed_tiles: Vec<(i32, i32)> = Vec::new();
        for l in self.lots.iter() {
            if !l.finished() {
                continue;
            }
            if self.district_at(l.x + l.w / 2, l.y + l.h / 2) != want {
                continue;
            }
            seed_tiles.push(door_tile(l));
            if seed_tiles.len() >= 6 {
                break;
            }
        }
        for s in seed_tiles {
            let Some(lane) = self.nearest_lane(s) else { continue };
            if let Some(run) = self.street_run_near(lane, 14, salt) {
                if run.len() >= 5 {
                    return Some(run);
                }
            }
        }
        None
    }

    /// Is `(x, y)` in an open green's pond, pool or fountain? (The market goes
    /// round the fountain; the shoppers do not wade.)
    fn in_open_water(&self, l: &Lot, x: f32, y: f32) -> bool {
        let civic = matches!(l.kind, LotKind::Landmark(LandmarkKind::Plaza));
        if !(l.kind == LotKind::Park || civic) {
            return self.map.at(x.floor() as i32, y.floor() as i32).is_water();
        }
        self.map.at(x.floor() as i32, y.floor() as i32).is_water()
            || map::in_park_water(
                l.w,
                l.h,
                civic,
                map::ParkKind::of_style(l.style),
                x - l.x as f32,
                y - l.y as f32,
            )
    }

    /// How far to step off a lane's centre line to be on the pavement: a full
    /// tile when the tile beside the street is walkable, otherwise a third of a
    /// tile (a street with no pavement, where the figures keep to the kerb).
    fn pave_step(&self, x: f32, y: f32, px: f32, py: f32) -> f32 {
        let tx = (x + px * 1.0).floor() as i32;
        let ty = (y + py * 1.0).floor() as i32;
        if self.map.in_bounds(tx, ty) {
            let k = self.map.at(tx, ty).kind;
            if k.is_walkable() && !k.is_lane() {
                return 0.95;
            }
        }
        0.32
    }

    /// The longest straight run of lane tiles through `p`, both directions.
    fn street_run_near(&self, p: (i32, i32), max: usize, _salt: u64) -> Option<Vec<(i32, i32)>> {
        let lane = self.nearest_lane(p)?;
        let mut best: Vec<(i32, i32)> = Vec::new();
        for axis in [(1, 0), (0, 1)] {
            let mut run = vec![lane];
            // Forward.
            let mut q = lane;
            for _ in 0..max {
                q = (q.0 + axis.0, q.1 + axis.1);
                if !self.is_lane(q) {
                    break;
                }
                run.push(q);
            }
            // Backward.
            let mut q = lane;
            let mut back: Vec<(i32, i32)> = Vec::new();
            for _ in 0..max {
                q = (q.0 - axis.0, q.1 - axis.1);
                if !self.is_lane(q) {
                    break;
                }
                back.push(q);
            }
            back.reverse();
            back.extend(run);
            if back.len() > best.len() {
                best = back;
            }
        }
        if best.len() < 3 {
            None
        } else {
            Some(best)
        }
    }

    fn is_lane(&self, p: (i32, i32)) -> bool {
        self.map.in_bounds(p.0, p.1) && self.map.at(p.0, p.1).kind.is_lane()
    }
}

/// The districts whose bins go out on `dow` (0 = Sunday). The round works the
/// town from its industrial edge on Monday to the high end on Friday.
fn bin_districts(dow: i32) -> &'static [District] {
    match dow.rem_euclid(7) {
        MONDAY => &[District::Industrial],
        TUESDAY => &[District::Core],
        WEDNESDAY => &[District::European],
        THURSDAY => &[District::Suburb],
        FRIDAY => &[District::Downtown, District::HighEnd],
        _ => &[],
    }
}

/// Outward and lateral unit steps for a lot's front (0 = +x, 1 = +y, 2 = -x,
/// 3 = -y).
fn outward(front: u8) -> (f32, f32) {
    match front % 4 {
        0 => (1.0, 0.0),
        1 => (0.0, 1.0),
        2 => (-1.0, 0.0),
        _ => (0.0, -1.0),
    }
}

fn lateral(front: u8) -> (f32, f32) {
    let (ox, oy) = outward(front);
    (-oy, ox)
}

fn dir_of(dx: f32, dy: f32) -> u8 {
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { 0 } else { 2 }
    } else if dy >= 0.0 {
        1
    } else {
        3
    }
}

fn hash3(a: u32, b: u32, c: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h
}

/// A slow vehicle working a street: drive to the end, pause, drive back.
/// `travelled` is in tiles from the start of the run.
fn pingpong(time: f64, span: f32, speed: f32, pause: f64) -> f32 {
    let span = span.max(0.0);
    if span <= 0.0 {
        return 0.0;
    }
    let drive = (span / speed.max(0.01)) as f64;
    let half = drive + pause;
    let q = time.rem_euclid(half * 2.0);
    if q < drive {
        (q * speed as f64) as f32
    } else if q < half {
        span
    } else if q < half + drive {
        span - ((q - half) * speed as f64) as f32
    } else {
        0.0
    }
}

fn run_len(route: &[(i32, i32)]) -> f32 {
    route
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs()) as f32)
        .sum()
}

/// A vehicle the week places on the road: everything the traffic layer needs to
/// pose and draw it, and nothing that the traffic step will move.
fn sched_vehicle(
    kind: VehicleKind,
    style: u8,
    route: Vec<(i32, i32)>,
    travelled: f32,
    salt: u64,
) -> Vehicle {
    let from = *route.first().unwrap_or(&(0, 0));
    let to = *route.last().unwrap_or(&(0, 0));
    Vehicle {
        kind,
        route,
        travelled,
        from,
        to,
        pace: 1.0,
        blink: 0.0,
        started: salt as f64,
        slot: None,
        variant: (salt % 8) as u8,
        style,
        signal: 0,
        braking: travelled <= 0.001,
        held: 0.0,
        stopped: false,
        hw: (0.0, 0.0),
        vel: 0.0,
    }
}

/// B77: the school year, as a map of the year the city is living in. A typical
/// northern-hemisphere year (England's, near enough for both sides of the
/// Atlantic): summer, Christmas, Easter plus its two-week break, and the three
/// half terms. The sim has no country, so "typical" is the honest answer.
pub fn school_holiday(year: i32, month: u8, day: u8) -> bool {
    let m = month as i32;
    let d = day as i32;
    match m {
        1 => d <= 4,                      // Christmas, into the new year
        2 => (12..=20).contains(&d),      // half term
        // March too: when Easter falls on 22 to 31 March (2016, 2024, 2027) the
        // fortnight around it starts in March; the arm used to begin in April.
        3 | 4 | 5 => {
            // Easter: the two weeks around the Sunday.
            let (em, ed) = easter(year);
            let day_of_year = yday(year, m, d);
            let easter_day = yday(year, em, ed);
            (day_of_year - easter_day).abs() <= 9
        }
        6 => d >= 27,                 // the summer term's last days
        7 | 8 => true,                // the summer holiday
        9 => d == 1,                  // back in September (the first is still off)
        10 => (24..=31).contains(&d), // half term
        11 => false,
        12 => d >= 19, // Christmas
        _ => false,
    }
}

fn yday(y: i32, m: i32, d: i32) -> i32 {
    const DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let mut n = d;
    for (i, v) in DAYS.iter().enumerate() {
        if i as i32 + 1 < m {
            n += v + if i == 1 && leap { 1 } else { 0 };
        }
    }
    n
}

/// Easter Sunday (Gauss/Anonymous Gregorian), as (month, day).
fn easter(y: i32) -> (i32, i32) {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    (month, day)
}
