//! B52: the glue between the sim's civic life (government, economy, pollution,
//! incidents, street life) and the renderer's civic layer.
//!
//! It lives in this crate because it is the one both the spike and the
//! wallpaper already depend on. One call, [`sync`], per frame: it compares the
//! sim's civic signature with what the scene was told and only rebuilds the
//! look when something changed (a ten-minute bucket of sim time, an election
//! phase, a fire's stage, the hour's street life). A quiet city costs a few
//! integer compares per frame.

use render::civic::{CivicDyn, CivicLook, CivicProp, CivicStatic, FireLook, PropKind, Stack};
use render::people::{Figure, POSE_PLAY, POSE_SIT, POSE_STAND, POSE_WAVE, POSE_WORK, PROP_CONFETTI, PROP_NONE};
use sim::civics::{FireStage, Phase};
use sim::landmarks::LandmarkKind;
use sim::streetlife::LifeKind;
use sim::{LotKind, Sim};

/// What the scene has been told.
#[derive(Default)]
pub struct Tracker {
    pub epoch: u64,
    key: u64,
}

fn fig(x: f32, y: f32, pose: u8, tone: u8, child: bool, dir: u8) -> Figure {
    Figure { x, y, pose, tone, child, prop: PROP_NONE, dir, open: true, lift: 0, frame: 0 }
}

fn hs(seed: u32, n: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ n.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65536.0
}

/// The nearest lane tile to a point (vehicles stand on the road).
fn lane_near(s: &Sim, at: (i32, i32)) -> Option<(f32, f32)> {
    for ring in 0..7i32 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dy.abs() != ring {
                    continue;
                }
                let (x, y) = (at.0 + dx, at.1 + dy);
                if s.map.in_bounds(x, y) && s.map.at(x, y).kind.is_lane() {
                    return Some((x as f32 + 0.5, y as f32 + 0.5));
                }
            }
        }
    }
    None
}

fn obj_of<'a>(objs: &'a [render::CityObj], x: i32, y: i32) -> Option<&'a render::CityObj> {
    objs.iter().find(|o| o.x == x && o.y == y)
}

/// Build the civic look for the sim's clock.
pub fn look(s: &Sim, objs: &[render::CityObj]) -> CivicLook {
    let mut stat = CivicStatic::default();
    let mut dy = CivicDyn::default();
    let hour = (s.time / 3600.0).rem_euclid(24.0) as f32;
    let seed = s.seed as u32;

    // --- the blackout's dark zone and its crew ------------------------------
    if let Some((cx, cy, r)) = s.blackout_zone() {
        // Quantised so the static layer re-bakes a few times as the crews win
        // the district back, not every step.
        stat.dark = Some((cx, cy, r.round() as i32));
        if let Some((x, y)) = s.blackout_crew() {
            dy.props.push(CivicProp { x, y, kind: PropKind::Truck, v: 0, dir: 0, lift: 0 });
            dy.props.push(CivicProp { x: x + 1.4, y, kind: PropKind::Barrier, v: 0, dir: 0, lift: 0 });
            dy.figures.push(fig(x + 0.9, y - 0.55, POSE_WORK, 8, false, 0));
            dy.figures.push(fig(x + 1.7, y - 0.5, POSE_STAND, 9, false, 2));
        }
    }

    // --- elections ------------------------------------------------------------
    let plaza = s.lots.iter().find(|l| l.finished() && l.kind == LotKind::Landmark(LandmarkKind::Plaza));
    let hall = s.lots.iter().find(|l| l.finished() && l.kind == LotKind::Hall);
    let phase = s.election_phase();
    if std::env::var("SIM_DEBUG").is_ok() {
        eprintln!("civic look: lots {} plaza {:?} hall {:?} phase {:?} t {:.0}", s.lots.len(), plaza.map(|l| (l.x, l.y, l.stage)), hall.map(|l| (l.x, l.y)), phase, s.time);
    }
    stat.rally = s.election_dress();
    if phase != Phase::None {
        // Posters on A-frames at the corners of the square and by the hall door.
        let mut spots: Vec<(f32, f32)> = Vec::new();
        if let Some(p) = plaza {
            let (x, y, w, h) = (p.x as f32, p.y as f32, p.w as f32, p.h as f32);
            spots.extend([(x - 0.2, y + h - 0.2), (x + w + 0.2, y + h - 0.2), (x + w * 0.5, y + h + 0.25), (x - 0.2, y + h * 0.5)]);
        }
        if let Some(l) = hall {
            let (dx, dy_) = sim::people::door_tile(l);
            spots.extend([(dx as f32 + 0.5, dy_ as f32 + 0.6), (dx as f32 - 1.1, dy_ as f32 + 0.5)]);
        }
        for (i, (x, y)) in spots.into_iter().enumerate() {
            dy.props.push(CivicProp { x, y, kind: PropKind::Poster, v: (i % 2) as u8, dir: 0, lift: 0 });
        }
    }
    if let Some(p) = plaza {
        let sq = render::spectacle::Square { x: p.x as f32 + 0.13, y: p.y as f32 + 0.13, w: p.w as f32 - 0.26, h: p.h as f32 - 0.26, level: p.level };
        let (sx0, sy0, sx1, sy1) = sq.stage_rect();
        match phase {
            Phase::Rally(u) => {
                // The crowd fills in and thins out; the candidates speak.
                let att = (u / 0.2).min(1.0) * (1.0 - ((u - 0.85).max(0.0) / 0.15)).clamp(0.0, 1.0);
                crowd(&mut dy.figures, &sq, att, seed, false);
                let mut sp = fig((sx0 + sx1) * 0.5 - 0.3, sy1 - 0.3, POSE_WAVE, 6, false, 1);
                sp.lift = 4;
                let mut sp2 = fig((sx0 + sx1) * 0.5 + 0.4, sy0 + 0.5, POSE_STAND, 2, false, 1);
                sp2.lift = 4;
                dy.figures.extend([sp, sp2]);
            }
            Phase::Night(u) => {
                let att = (u / 0.15).min(1.0) * (1.0 - ((u - 0.9).max(0.0) / 0.1)).clamp(0.0, 1.0);
                // The result is read at 21:30, 43 per cent through the night.
                let cheering = u > 0.43;
                crowd(&mut dy.figures, &sq, 0.8 * att, seed ^ 0x77, cheering);
                let lead = ((u - 0.1) / 0.35).clamp(0.0, 1.0);
                let (bx, by) = ((sx0 + sx1) * 0.5 - 1.7, sy0 + 0.9);
                dy.props.push(CivicProp { x: bx, y: by, kind: PropKind::Board, v: (lead * 255.0) as u8, dir: 0, lift: 0 });
                let mut sp = fig((sx0 + sx1) * 0.5, sy1 - 0.3, if cheering { POSE_WAVE } else { POSE_STAND }, 6, false, 1);
                sp.lift = 4;
                dy.figures.push(sp);
            }
            _ => {}
        }
    }
    // Voting day: a queue at the hall door.
    if phase == Phase::Voting {
        if let Some(l) = hall {
            let (dx, dy_) = sim::people::door_tile(l);
            let n = 5 + (hs(seed, dx as u32) * 4.0) as u32;
            let along = if l.front % 2 == 0 { (0.0, 1.0) } else { (1.0, 0.0) };
            for i in 0..n {
                let off = 0.7 + i as f32 * 0.42;
                dy.figures.push(fig(dx as f32 + 0.5 + along.0 * off, dy_ as f32 + 0.5 + along.1 * off, POSE_STAND, (hs(seed, i * 5) * 8.0) as u8 % 8, false, (l.front + 2) % 4));
            }
        }
    }

    // --- fire --------------------------------------------------------------
    if let Some((f, st)) = s.fire_stage() {
        if let Some(l) = s.lots.iter().find(|l| l.x == f.x && l.y == f.y) {
            let wall = obj_of(objs, f.x, f.y).map_or(24, |o| o.wall_height());
            let (stage, u) = match st {
                FireStage::Smoke(u) => (0, u),
                FireStage::Burning(u) => (1, u),
                FireStage::Aftermath(u) => (2, u),
            };
            dy.fires.push(FireLook { x: l.x, y: l.y, w: l.w, h: l.h, level: l.level, wall, stage, u });
            let door = sim::people::door_tile(l);
            if let Some((ex, ey)) = lane_near(s, door) {
                let along_x = !matches!(l.front % 4, 0 | 2);
                if stage == 1 && s.time - f.t0 > 1500.0 {
                    dy.props.push(CivicProp { x: ex, y: ey, kind: PropKind::Engine, v: 0, dir: if along_x { 0 } else { 1 }, lift: 0 });
                    // Firefighters at the hose.
                    dy.figures.push(fig(ex + 0.2, ey - 0.7, POSE_WORK, 8, false, 1));
                    dy.figures.push(fig(ex + 0.9, ey - 0.8, POSE_WORK, 9, false, 1));
                } else if stage == 2 {
                    dy.props.push(CivicProp { x: ex, y: ey, kind: PropKind::Truck, v: 0, dir: if along_x { 0 } else { 1 }, lift: 0 });
                    dy.props.push(CivicProp { x: door.0 as f32 + 0.5, y: door.1 as f32 + 0.7, kind: PropKind::Barrier, v: 0, dir: 0, lift: 0 });
                    dy.figures.push(fig(door.0 as f32 - 0.3, door.1 as f32 + 0.4, POSE_WORK, 8, false, 1));
                    dy.figures.push(fig(door.0 as f32 + 1.3, door.1 as f32 + 0.5, POSE_STAND, 9, false, 3));
                }
            }
        }
    }

    // --- the police car and the post van ----------------------------------------
    for l in s.lots.iter().filter(|l| l.finished()) {
        let (kind, from, to) = match l.kind {
            LotKind::Landmark(LandmarkKind::PoliceStation) => (PropKind::PoliceCar, 0.0, 24.0),
            LotKind::Landmark(LandmarkKind::PostOffice) => (PropKind::PostVan, 7.0, 17.0),
            _ => continue,
        };
        if hour < from || hour >= to {
            continue;
        }
        if let Some((x, y)) = lane_near(s, sim::people::door_tile(l)) {
            dy.props.push(CivicProp { x, y, kind, v: 0, dir: if matches!(l.front % 4, 0 | 2) { 1 } else { 0 }, lift: 0 });
        }
    }

    // --- chimneys and smog -----------------------------------------------------
    let works: Vec<&sim::Lot> = s.lots.iter().filter(|l| l.finished() && l.kind == LotKind::Industry).collect();
    if !works.is_empty() {
        // The haze hangs over the works the frame can see (the average of all
        // of them may lie off the screen).
        let (ox, oy, vw, vh) = s.viewport;
        let seen: Vec<&&sim::Lot> = works
            .iter()
            .filter(|l| {
                let (sx, sy) = (ox + (l.x - l.y) * 32, oy + (l.x + l.y) * 16);
                sx > 0 && sx < vw && sy > vh / 4 && sy < vh
            })
            .collect();
        let pool: Vec<&sim::Lot> = if seen.is_empty() { works.clone() } else { seen.into_iter().copied().collect() };
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for l in &pool {
            cx += l.x as f32 + l.w as f32 * 0.5;
            cy += l.y as f32 + l.h as f32 * 0.5;
        }
        dy.smog_at = (cx / pool.len() as f32, cy / pool.len() as f32);
        dy.smog = s.smog();
        // The works smoke while the shift is in.
        if (5.5..20.5).contains(&hour) {
            let scrubbed = s.civics.cleanups & sim::civics::CLEAN_SCRUBBERS != 0;
            for l in works.iter().take(8) {
                let wall = obj_of(objs, l.x, l.y).map_or(20, |o| o.wall_height());
                dy.stacks.push(Stack {
                    x: l.x as f32 + l.w as f32 * 0.8,
                    y: l.y as f32 + l.h as f32 * 0.3,
                    level: l.level,
                    wall,
                    strength: (0.25 + s.smog() * 0.75).min(1.0),
                    scrubbed,
                });
            }
        }
    }

    // --- street life ------------------------------------------------------------
    for it in s.street_life() {
        match it.kind {
            LifeKind::Band => {
                for i in 0..4 {
                    let x = it.x - 0.95 + i as f32 * 0.62;
                    dy.figures.push(fig(x, it.y, POSE_STAND, 14, false, 3));
                }
                dy.props.push(CivicProp { x: it.x, y: it.y, kind: PropKind::Notes, v: 0, dir: 0, lift: 6 });
            }
            LifeKind::Sax => {
                dy.figures.push(fig(it.x, it.y, POSE_STAND, 3, false, it.dir));
                dy.props.push(CivicProp { x: it.x, y: it.y, kind: PropKind::Sax, v: 0, dir: it.dir, lift: 0 });
            }
            LifeKind::Cat => {
                if let Some(o) = obj_of(objs, it.x as i32, it.y as i32) {
                    // On the roof's front corner, at the height of the eaves.
                    let lift = o.wall_height() + 5;
                    dy.props.push(CivicProp {
                        x: it.x + it.w as f32 - 0.55,
                        y: it.y + it.h as f32 - 0.55,
                        kind: PropKind::Cat,
                        v: (it.seed % 4) as u8,
                        dir: it.dir,
                        lift,
                    });
                }
            }
            LifeKind::Lemonade => {
                dy.props.push(CivicProp { x: it.x, y: it.y, kind: PropKind::Lemonade, v: 0, dir: it.dir, lift: 0 });
                dy.figures.push(fig(it.x - 0.55, it.y + 0.45, POSE_STAND, 4, true, 3));
                dy.figures.push(fig(it.x + 0.45, it.y + 0.5, POSE_PLAY, 5, true, 3));
            }
            LifeKind::Angler => {
                dy.figures.push(fig(it.x, it.y, POSE_SIT, 2 + (it.seed % 4) as u8, false, it.dir));
                dy.props.push(CivicProp { x: it.x, y: it.y, kind: PropKind::Rod, v: 0, dir: it.dir, lift: 0 });
            }
        }
    }
    CivicLook { stat, dynamic: dy }
}

/// A crowd in the square, facing the stage.
fn crowd(v: &mut Vec<Figure>, sq: &render::spectacle::Square, att: f32, seed: u32, cheering: bool) {
    let (sx0, _sy0, sx1, sy1) = sq.stage_rect();
    let (fx, fy) = (sq.x + sq.w * 0.5, sq.y + sq.h * 0.5);
    let max = ((sq.w * sq.h) * 2.6) as usize;
    let want = (max as f32 * att) as usize;
    let mut placed = 0;
    let mut i = 0u32;
    while placed < want && i < 500 {
        i += 1;
        let h1 = hs(seed, i * 3);
        let h2 = hs(seed, i * 3 + 1);
        let (x, y) = if hs(seed, i * 3 + 2) < 0.65 {
            (sx0 - 0.3 + h1 * (sx1 - sx0 + 0.6), sy1 + 0.45 + h2 * 1.8)
        } else {
            (sq.x + 0.5 + h1 * (sq.w - 1.0), sy1 + 0.5 + h2 * (sq.y + sq.h - 0.45 - (sy1 + 0.5)))
        };
        if y > sq.y + sq.h - 0.4 || ((x - fx).powi(2) + (y - fy).powi(2)).sqrt() < 1.1 {
            continue;
        }
        let child = hs(seed, i * 7) < 0.14;
        let tone = (hs(seed, i * 11) * 8.0) as u8 % 8;
        let wave = cheering && hs(seed, i * 13) < 0.5;
        let mut f = fig(x, y, if wave { POSE_WAVE } else { POSE_STAND }, tone, child, 3);
        if wave && hs(seed, i * 17) < 0.4 {
            f.prop = PROP_CONFETTI;
        }
        v.push(f);
        placed += 1;
    }
}

/// Is a fresh look due? (Ten sim minutes on, or the civic epoch moved.) Cheap:
/// the wallpaper asks every frame and only builds the city's objects when yes.
pub fn due(s: &Sim, tr: &Tracker) -> bool {
    let epoch = s.civic_epoch();
    let key = ((s.time / 600.0) as u64).wrapping_mul(0x9E37_79B9) ^ epoch;
    epoch != tr.epoch || key != tr.key
}

/// Tell the scene what the civic layer looks like now. Returns `true` when the
/// city's sprites must be attached again (a lot became vacant, burnt or dark).
pub fn sync(scene: &mut render::Scene, s: &Sim, objs: &[render::CityObj], tr: &mut Tracker) -> bool {
    let epoch = s.civic_epoch();
    let rebuild = epoch != tr.epoch;
    tr.epoch = epoch;
    tr.key = ((s.time / 600.0) as u64).wrapping_mul(0x9E37_79B9) ^ epoch;
    scene.set_civic(look(s, objs));
    scene.civic_tick();
    rebuild
}
