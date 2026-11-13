//! B43c: building sites that read at 1x. The static half: everything a plot
//! shows in the baked layer while it is being built (survey stakes, orange
//! fence, the dug pit, the slab and its rebar, the steel or timber frame going
//! up, the clad walls behind scaffolding and safety netting, the scaffold
//! coming down). The moving half — cranes, the excavator, trucks, the cabin —
//! is `crate::rigs`, drawn per frame.
//!
//! Every stage is a pure function of the plot (`CityObj`) so a stage change
//! repaints exactly the plot's own tiles.

use super::{walls, CityObj, Finish, Stage};
use crate::framebuffer::Framebuffer;
use crate::palette::{self, pal, Color};

pub type Proj<'a> = &'a dyn Fn(f32, f32, i32) -> (f32, f32);

/// Plot geometry shared by the stage painters.
pub struct Geo {
    pub lot: (f32, f32, f32, f32),
    pub foot: (f32, f32, f32, f32),
    /// Finished wall height in px.
    pub h: i32,
    /// Steel and concrete (mid-rise, tower, industry) or timber (cottage).
    pub steel: bool,
    /// Pixels per storey.
    pub floor: i32,
    pub front: u8,
    pub seed: u32,
}

impl Geo {
    pub fn of(o: &CityObj, lot: (f32, f32, f32, f32), foot: (f32, f32, f32, f32)) -> Geo {
        let massing = o.kind.is_massing();
        let floor = if massing {
            if o.storeys >= 8 { 10 } else { 8 }
        } else {
            18
        };
        Geo {
            lot,
            foot,
            h: o.wall_height(),
            steel: massing,
            floor,
            front: o.front,
            seed: o.seed,
        }
    }
}

fn hash(seed: u32, k: u32) -> f32 {
    let mut x = seed ^ k.wrapping_mul(0x9E37_79B9);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2545_F491);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65535.0
}

fn pt(p: Proj, x: f32, y: f32, z: i32) -> (i32, i32) {
    let q = p(x, y, z);
    (q.0.round() as i32, q.1.round() as i32)
}

/// The yard while the plot is a site: trampled soil with gravel and wheel ruts
/// instead of the neat lawn (which is laid when the building is finished).
pub fn ground(fb: &mut Framebuffer, p: Proj, g: &Geo, stage: Stage) {
    if matches!(stage, Stage::Staked) {
        return;
    }
    let (x0, y0, x1, y1) = g.lot;
    let quad = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y1, 0), p(x0, y1, 0)];
    fb.fill_poly(&quad, palette::shade(pal::DIRT[4], 0.96));
    // Mottling: darker soil, pale gravel, a few stones.
    let n = ((x1 - x0) * (y1 - y0) * 26.0) as u32;
    for i in 0..n {
        let u = x0 + hash(g.seed, i * 3 + 1) * (x1 - x0);
        let v = y0 + hash(g.seed, i * 3 + 2) * (y1 - y0);
        let c = match (hash(g.seed, i * 3 + 3) * 5.0) as u32 {
            0 | 1 => pal::DIRT[2],
            2 => pal::DIRT[6],
            3 => pal::GRAVEL,
            _ => pal::DIRT[1],
        };
        let (sx, sy) = pt(p, u, v, 0);
        fb.put(sx, sy, c);
        if i % 4 == 0 {
            fb.put(sx + 1, sy, c);
        }
    }
    // Wheel ruts from the gate into the plot.
    let (gx, gy, ix, iy) = gate(g);
    for off in [-0.09f32, 0.09] {
        let (ax, ay) = if g.front % 2 == 0 { (gx, gy + off) } else { (gx + off, gy) };
        let (bx, by) = if g.front % 2 == 0 { (ix, iy + off) } else { (ix + off, iy) };
        fb.line(p(ax, ay, 0), p(bx, by, 0), palette::shade(pal::DIRT[1], 0.9));
    }
}

/// The gate: (on the lot edge, a point 0.8 tile inside).
fn gate(g: &Geo) -> (f32, f32, f32, f32) {
    let (x0, y0, x1, y1) = g.lot;
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    match g.front % 4 {
        0 => (x1, cy, x1 - 0.8, cy),
        1 => (cx, y1, cx, y1 - 0.8),
        2 => (x0, cy, x0 + 0.8, cy),
        _ => (cx, y0, cx, y0 + 0.8),
    }
}

/// One run of orange mesh fence between two ground points.
fn fence_edge(fb: &mut Framebuffer, p: Proj, a: (f32, f32), b: (f32, f32), gate: bool) {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let pa = p(a.0, a.1, 0);
    let pb = p(b.0, b.1, 0);
    let n = (pb.0 - pa.0).abs().max((pb.1 - pa.1).abs()).ceil().max(2.0) as i32;
    let posts = (len * 2.0).round().max(1.0) as i32;
    let gap = |t: f32| gate && (t * len - len * 0.5).abs() < 0.42;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        if gap(t) {
            continue;
        }
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (sx, sy) = pt(p, x, y, 0);
        let alt = i % 2 == 0;
        fb.put(sx, sy - 5, pal::FENCE);
        fb.put(sx, sy - 4, if alt { pal::FENCE } else { pal::FENCE_D });
        fb.put(sx, sy - 3, if alt { pal::FENCE_D } else { pal::FENCE });
        fb.put(sx, sy - 2, if alt { pal::FENCE } else { pal::FENCE_D });
        fb.put(sx, sy - 1, pal::FENCE_D);
    }
    for k in 0..=posts {
        let t = k as f32 / posts as f32;
        if gap(t) {
            continue;
        }
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        let (sx, sy) = pt(p, x, y, 0);
        for z in 0..=6 {
            fb.put(sx, sy - z, pal::FENCE_POST);
        }
        fb.put(sx, sy - 7, palette::shade(pal::FENCE_POST, 1.35));
    }
}

/// The two fence runs behind the building (drawn before it).
pub fn fence_back(fb: &mut Framebuffer, p: Proj, g: &Geo, stage: Stage) {
    if matches!(stage, Stage::Staked) {
        return;
    }
    let (x0, y0, x1, y1) = g.lot;
    fence_edge(fb, p, (x0, y0), (x1, y0), g.front % 4 == 3);
    fence_edge(fb, p, (x0, y0), (x0, y1), g.front % 4 == 2);
}

/// The two fence runs in front of it (drawn after).
pub fn fence_front(fb: &mut Framebuffer, p: Proj, g: &Geo, stage: Stage) {
    let (x0, y0, x1, y1) = g.lot;
    if matches!(stage, Stage::Staked) {
        // Only the street side is fenced off at first.
        match g.front % 4 {
            1 => fence_edge(fb, p, (x0, y1), (x1, y1), true),
            0 => fence_edge(fb, p, (x1, y0), (x1, y1), true),
            _ => {}
        }
        return;
    }
    fence_edge(fb, p, (x0, y1), (x1, y1), g.front % 4 == 1);
    fence_edge(fb, p, (x1, y0), (x1, y1), g.front % 4 == 0);
}

fn stake(fb: &mut Framebuffer, p: Proj, x: f32, y: f32) {
    let (sx, sy) = pt(p, x, y, 0);
    for z in 0..6 {
        fb.put(sx, sy - z, pal::STAKE);
    }
    fb.put(sx, sy - 6, pal::STAKE_TAPE);
    fb.put(sx, sy - 7, pal::STAKE_TAPE);
    fb.put(sx + 1, sy - 6, palette::shade(pal::STAKE_TAPE, 0.8));
}

/// Stage 0: the surveyed lot — stakes at the corners and along the edges, string
/// between them, a tripod and its instrument.
pub fn staked(fb: &mut Framebuffer, p: Proj, g: &Geo) {
    let (x0, y0, x1, y1) = g.foot;
    // String first so the stakes stand over it.
    let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    for i in 0..4 {
        let a = c[i];
        let b = c[(i + 1) % 4];
        fb.line(p(a.0, a.1, 5), p(b.0, b.1, 5), pal::ROPE);
    }
    for i in 0..4 {
        let a = c[i];
        let b = c[(i + 1) % 4];
        stake(fb, p, a.0, a.1);
        stake(fb, p, (a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    }
    // Surveyor's tripod and instrument.
    let (tx, ty) = ((x0 + x1) * 0.5 + 0.1, (y0 + y1) * 0.5 - 0.05);
    let apex = pt(p, tx, ty, 11);
    for (dx, dy) in [(-0.16f32, 0.05f32), (0.14, 0.1), (0.0, -0.17)] {
        let foot = pt(p, tx + dx, ty + dy, 0);
        fb.line(
            (apex.0 as f32, apex.1 as f32),
            (foot.0 as f32, foot.1 as f32),
            palette::shade(pal::TIMBER, 0.9),
        );
    }
    fb.fill_rect(apex.0 - 1, apex.1 - 3, 3, 3, pal::CRANE);
    fb.put(apex.0, apex.1 - 4, pal::STEEL_D);
    // A bundle of stakes beside the corner.
    let (hx, hy) = pt(p, x0 + 0.1, y1 - 0.05, 0);
    for k in 0..4 {
        fb.line(
            (hx as f32 - 2.0 + k as f32 * 0.5, hy as f32),
            (hx as f32 + 3.0 - k as f32 * 0.5, hy as f32 - 2.0 - (k % 2) as f32),
            pal::STAKE,
        );
    }
}

/// Stage 1: excavation. The footprint is dug out: lit and shaded earth walls,
/// a dark floor, the front lip, and a pile of spoil.
pub fn excavation(fb: &mut Framebuffer, p: Proj, g: &Geo) {
    let (fx0, fy0, fx1, fy1) = g.foot;
    let i = 0.2;
    let (x0, y0, x1, y1) = (fx0 + i, fy0 + i, fx1 - i, fy1 - i);
    let d = 9;
    // Raw earth rim, a little wider than the hole.
    let m = 0.09;
    let rim = [p(x0 - m, y0 - m, 0), p(x1 + m, y0 - m, 0), p(x1 + m, y1 + m, 0), p(x0 - m, y1 + m, 0)];
    fb.fill_poly(&rim, pal::DIRT[6]);
    let floor = [p(x0, y0, -d), p(x1, y0, -d), p(x1, y1, -d), p(x0, y1, -d)];
    fb.fill_poly(&floor, pal::DIRT[0]);
    // Far walls: the one facing +y is lit, the one facing +x in shade.
    let wa = [p(x0, y0, 0), p(x1, y0, 0), p(x1, y0, -d), p(x0, y0, -d)];
    fb.fill_poly(&wa, pal::DIRT[3]);
    let wb = [p(x0, y0, 0), p(x0, y1, 0), p(x0, y1, -d), p(x0, y0, -d)];
    fb.fill_poly(&wb, pal::DIRT[2]);
    for z in [-3, -6] {
        fb.line(p(x0, y0, z), p(x1, y0, z), palette::shade(pal::DIRT[3], 0.82));
        fb.line(p(x0, y0, z), p(x0, y1, z), palette::shade(pal::DIRT[2], 0.82));
    }
    // Wet patch and stones on the floor.
    let (wx, wy) = pt(p, (x0 + x1) * 0.5, (y0 + y1) * 0.5, -d);
    fb.fill_rect(wx - 3, wy - 1, 6, 2, palette::mix(pal::DIRT[0], palette::rgb(0x3a, 0x4a, 0x5c), 0.55));
    for k in 0..6u32 {
        let u = x0 + 0.1 + hash(g.seed, 40 + k) * (x1 - x0 - 0.2);
        let v = y0 + 0.1 + hash(g.seed, 60 + k) * (y1 - y0 - 0.2);
        let (sx, sy) = pt(p, u, v, -d);
        fb.put(sx, sy, pal::BOULDER[(k % 2) as usize]);
    }
    // The front lips hide the overhang of the floor.
    let la = [p(x0, y1, 0), p(x1, y1, 0), p(x1, y1, -d), p(x0, y1, -d)];
    fb.fill_poly(&la, pal::DIRT[5]);
    let lb = [p(x1, y0, 0), p(x1, y1, 0), p(x1, y1, -d), p(x1, y0, -d)];
    fb.fill_poly(&lb, pal::DIRT[4]);
    fb.line(p(x0, y1, 0), p(x1, y1, 0), pal::DIRT[6]);
    fb.line(p(x1, y0, 0), p(x1, y1, 0), pal::DIRT[6]);
    // The spoil heap at the right-hand corner of the plot.
    let (hx, hy) = pt(p, fx1 - 0.05, fy0 + 0.12, 0);
    mound(fb, hx, hy, 10, 7, [pal::DIRT[5], pal::DIRT[3], pal::DIRT[1]]);
}

/// A half-ellipse heap: light from the upper left.
fn mound(fb: &mut Framebuffer, cx: i32, cy: i32, rx: i32, ry: i32, c: [Color; 3]) {
    for dy in 0..ry {
        let t = dy as f32 / ry as f32;
        let half = (rx as f32 * (1.0 - t * t).max(0.0).sqrt()) as i32;
        for dx in -half..=half {
            let col = if dx < -half / 3 - (dy / 3) {
                c[0]
            } else if dx > half / 3 {
                c[2]
            } else {
                c[1]
            };
            fb.put(cx + dx, cy - dy, col);
        }
    }
    fb.put(cx - rx / 2, cy - 1, c[2]);
    fb.put(cx + rx / 3, cy - ry / 2, c[0]);
}

/// Stage 2: the slab. Timber formwork round the edge, grey concrete with a
/// rebar mesh, rust-coloured starter bars standing out of it.
pub fn slab(fb: &mut Framebuffer, p: Proj, g: &Geo) {
    slab_with(fb, p, g, true);
}

/// The slab with or without its timber formwork (the frame stage stands on a
/// stripped slab, so the studs are not lost against the boards).
pub fn slab_with(fb: &mut Framebuffer, p: Proj, g: &Geo, formwork: bool) {
    let (x0, y0, x1, y1) = g.foot;
    let zt = 4;
    let (fl, fr) = if formwork { (pal::TIMBER, pal::TIMBER) } else { (pal::CONCRETE, pal::CONCRETE) };
    let left = [p(x0, y1, zt), p(x1, y1, zt), p(x1, y1, 0), p(x0, y1, 0)];
    fb.fill_poly(&left, palette::shade(fl, if formwork { 0.95 } else { 0.85 }));
    let right = [p(x1, y0, zt), p(x1, y1, zt), p(x1, y1, 0), p(x1, y0, 0)];
    fb.fill_poly(&right, palette::shade(fr, if formwork { 0.7 } else { 0.62 }));
    if formwork {
        fb.line(p(x0, y1, 2), p(x1, y1, 2), palette::shade(pal::TIMBER, 0.72));
        fb.line(p(x1, y0, 2), p(x1, y1, 2), palette::shade(pal::TIMBER, 0.52));
    }
    let top = [p(x0, y0, zt), p(x1, y0, zt), p(x1, y1, zt), p(x0, y1, zt)];
    fb.fill_poly(&top, palette::shade(pal::CONCRETE, 1.04));
    let step = 0.5;
    let mut u = x0 + step;
    while u < x1 - 0.01 {
        fb.line(p(u, y0, zt), p(u, y1, zt), palette::shade(pal::CONCRETE, 0.9));
        u += step;
    }
    let mut v = y0 + step;
    while v < y1 - 0.01 {
        fb.line(p(x0, v, zt), p(x1, v, zt), palette::shade(pal::CONCRETE, 0.9));
        v += step;
    }
    let mut u = x0 + step;
    while u < x1 - 0.01 {
        let mut v = y0 + step;
        while v < y1 - 0.01 {
            let (sx, sy) = pt(p, u, v, zt);
            fb.put(sx, sy, pal::REBAR);
            v += step;
        }
        u += step;
    }
    fb.line(p(x0, y1, zt), p(x1, y1, zt), palette::shade(pal::CONCRETE, 1.2));
    fb.line(p(x1, y0, zt), p(x1, y1, zt), palette::shade(pal::CONCRETE, 1.1));
    // Starter bars round the edge.
    for (ax, ay, bx, by) in [(x0, y0, x1, y0), (x0, y0, x0, y1), (x0, y1, x1, y1), (x1, y0, x1, y1)] {
        let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        let n = (len / 0.5).round().max(1.0) as i32;
        for k in 0..=n {
            let t = k as f32 / n as f32;
            let (sx, sy) = pt(p, ax + (bx - ax) * t, ay + (by - ay) * t, zt);
            for z in 0..5 {
                fb.put(sx, sy - z, pal::REBAR);
            }
            fb.put(sx, sy - 5, palette::shade(pal::REBAR, 1.25));
        }
    }
}

/// How far up the frame is at the framing stage: about 60 % of the height,
/// rounded to whole storeys (at least one).
pub fn frame_height(g: &Geo) -> i32 {
    if !g.steel {
        // A cottage's studs go right up to the plate.
        return ((g.h as f32 * 0.85) as i32).max(10);
    }
    let floors = (g.h as f32 / g.floor as f32).round().max(1.0);
    let done = (floors * 0.6).round().max(1.0);
    (done as i32 * g.floor).min(g.h)
}

/// A steel or timber skeleton from `from` to `hf`.
pub fn frame(fb: &mut Framebuffer, p: Proj, g: &Geo, from: i32, hf: i32) {
    if g.steel {
        frame_steel(fb, p, g, from, hf);
    } else {
        frame_timber(fb, p, g, from, hf);
    }
}

fn columns(g: &Geo) -> (Vec<f32>, Vec<f32>) {
    let (x0, y0, x1, y1) = g.foot;
    let nx = ((x1 - x0) / 0.9).round().max(1.0) as usize;
    let ny = ((y1 - y0) / 0.9).round().max(1.0) as usize;
    (
        (0..=nx).map(|i| x0 + (x1 - x0) * i as f32 / nx as f32).collect(),
        (0..=ny).map(|i| y0 + (y1 - y0) * i as f32 / ny as f32).collect(),
    )
}

fn frame_steel(fb: &mut Framebuffer, p: Proj, g: &Geo, from: i32, hf: i32) {
    let (x0, y0, x1, y1) = g.foot;
    let (cx, cy) = columns(g);
    let steel = pal::STEEL;
    let dark = pal::STEEL_D;
    for &x in &cx {
        fb.line(p(x, y0, from), p(x, y0, hf), dark);
    }
    for &y in &cy {
        fb.line(p(x0, y, from), p(x0, y, hf), dark);
    }
    let floors = ((hf - from) / g.floor.max(1)).max(1);
    let shade_in = palette::rgb(0x2a, 0x28, 0x30);
    for k in 1..=floors {
        let z0 = from + g.floor * (k - 1) + if k == 1 && from == 0 { 4 } else { 0 };
        let z = (from + g.floor * k).min(hf);
        let lq = [p(x0, y1, z), p(x1, y1, z), p(x1, y1, z0), p(x0, y1, z0)];
        fb.fill_poly_blended(&lq, shade_in, 0.55);
        let rq = [p(x1, y0, z), p(x1, y1, z), p(x1, y1, z0), p(x1, y0, z0)];
        fb.fill_poly_blended(&rq, shade_in, 0.65);
        let deck = [p(x0, y0, z), p(x1, y0, z), p(x1, y1, z), p(x0, y1, z)];
        let top = k == floors;
        fb.fill_poly(&deck, palette::shade(pal::CONCRETE, if top { 1.0 } else { 0.88 }));
        let le = [p(x0, y1, z), p(x1, y1, z), p(x1, y1, z - 2), p(x0, y1, z - 2)];
        fb.fill_poly(&le, palette::shade(pal::CONCRETE, 0.8));
        let re = [p(x1, y0, z), p(x1, y1, z), p(x1, y1, z - 2), p(x1, y0, z - 2)];
        fb.fill_poly(&re, palette::shade(pal::CONCRETE, 0.6));
        for b in 0..cx.len() - 1 {
            if hash(g.seed, (k * 17 + b as i32) as u32) < 0.45 {
                fb.line(p(cx[b], y1, z0), p(cx[b + 1], y1, z - 2), dark);
            }
        }
        for b in 0..cy.len() - 1 {
            if hash(g.seed, (k * 29 + b as i32 + 90) as u32) < 0.45 {
                fb.line(p(x1, cy[b], z0), p(x1, cy[b + 1], z - 2), palette::shade(dark, 0.8));
            }
        }
    }
    for &x in &cx {
        fb.line(p(x, y1, from), p(x, y1, hf), steel);
    }
    for &y in &cy {
        fb.line(p(x1, y, from), p(x1, y, hf), palette::shade(steel, 0.8));
    }
    // Starter columns above the top deck.
    for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
        fb.line(p(x, y, hf), p(x, y, hf + 8), steel);
        let (sx, sy) = pt(p, x, y, hf + 8);
        fb.put(sx - 1, sy, dark);
        fb.put(sx + 1, sy, dark);
    }
}

fn frame_timber(fb: &mut Framebuffer, p: Proj, g: &Geo, from: i32, hf: i32) {
    let (x0, y0, x1, y1) = g.foot;
    let tim = pal::TIMBER;
    let dark_in = palette::rgb(0x2c, 0x26, 0x24);
    for (x, y) in [(x0, y0), (x1, y0), (x0, y1)] {
        fb.line(p(x, y, from), p(x, y, hf), palette::shade(tim, 0.7));
    }
    fb.line(p(x0, y0, hf), p(x1, y0, hf), palette::shade(tim, 0.8));
    fb.line(p(x0, y0, hf), p(x0, y1, hf), palette::shade(tim, 0.8));
    for (is_left, lum) in [(true, 1.0f32), (false, 0.72)] {
        let (a0, a1, fixed) = if is_left { (x0, x1, y1) } else { (y0, y1, x1) };
        let q = |a: f32, z: i32| if is_left { p(a, fixed, z) } else { p(fixed, a, z) };
        let quad = [q(a0, hf), q(a1, hf), q(a1, from), q(a0, from)];
        fb.fill_poly_blended(&quad, dark_in, 0.6);
        let bays = ((a1 - a0) / 0.5).round().max(1.0) as usize;
        for b in 0..bays {
            let ba = a0 + (a1 - a0) * b as f32 / bays as f32;
            let bb = a0 + (a1 - a0) * (b + 1) as f32 / bays as f32;
            let h = hash(g.seed, (b as u32) * 7 + if is_left { 3 } else { 5 });
            if h < 0.1 {
                let pq = [q(ba, hf), q(bb, hf), q(bb, from), q(ba, from)];
                fb.fill_poly(&pq, palette::shade(pal::SHEATHING, lum));
            } else if h < 0.45 {
                fb.line(q(ba, from), q(bb, hf), palette::shade(tim, lum * 0.85));
            }
        }
        for b in 0..=bays {
            let a = a0 + (a1 - a0) * b as f32 / bays as f32;
            let stud = palette::shade(pal::SHEATHING, lum * 1.05);
            fb.line(q(a, from), q(a, hf), stud);
            fb.line(q(a, from), q(a, hf), stud);
            let (ax, ay) = (q(a, from).0, q(a, from).1);
            let (bx, by) = (q(a, hf).0, q(a, hf).1);
            fb.line((ax + 1.0, ay), (bx + 1.0, by), palette::shade(tim, lum * 0.8));
        }
        for z in [hf, hf - 1, from + (hf - from) / 2] {
            fb.line(q(a0, z), q(a1, z), palette::shade(tim, lum * 1.08));
        }
    }
}

/// Stage 4: enclosure. The lower part is clad (walls with no windows yet),
/// the upper part is still bare frame, and scaffolding with netting stands
/// against both.
pub fn enclosure(fb: &mut Framebuffer, p: Proj, g: &Geo, wall: Color, roof: Color, glow: f32) {
    let (x0, y0, x1, y1) = g.foot;
    let floors = (g.h as f32 / g.floor as f32).round().max(1.0) as i32;
    let clad = (((floors as f32 * 0.6).round() as i32).clamp(1, floors) * g.floor).min(g.h);
    walls(
        fb,
        &p,
        (x0, y0, x1, y1),
        0,
        clad,
        &Finish { wall, roof: pal::CONCRETE, style: 5, seed: g.seed, glow, hour: 12.0, obj: crate::windows::obj_key(g.lot.0 as i32, g.lot.1 as i32), floor_px: g.floor, windows: false },
    );
    let _ = roof;
    // The bare upper frame rises out of the clad part.
    if clad < g.h {
        frame(fb, p, g, clad, g.h);
    }
    // A dark band of membrane at the top of the clad part.
    fb.line(p(x0, y1, clad - 1), p(x1, y1, clad - 1), palette::shade(pal::TRACK, 1.6));
    fb.line(p(x1, y0, clad - 1), p(x1, y1, clad - 1), palette::shade(pal::TRACK, 1.3));
    if !g.steel {
        roof_trusses(fb, p, g, clad);
    }
    scaffold(fb, p, g, clad + 10, true, true, true);
}

/// Bare timber roof trusses over the clad walls of a cottage: a ridge beam and
/// a rafter pair every half tile.
fn roof_trusses(fb: &mut Framebuffer, p: Proj, g: &Geo, z: i32) {
    let (x0, y0, x1, y1) = g.foot;
    let ym = (y0 + y1) * 0.5;
    let rise = 10;
    let tim = pal::TIMBER;
    let n = (((x1 - x0) / 0.5).round() as i32).max(1);
    for k in 0..=n {
        let x = x0 + (x1 - x0) * k as f32 / n as f32;
        fb.line(p(x, y1, z), p(x, ym, z + rise), palette::shade(tim, 1.05));
        fb.line(p(x, y0, z), p(x, ym, z + rise), palette::shade(tim, 0.75));
    }
    fb.line(p(x0, ym, z + rise), p(x1, ym, z + rise), palette::shade(tim, 1.15));
}

/// Rafters standing on the top plate of a cottage frame.
pub fn rafters(fb: &mut Framebuffer, p: Proj, g: &Geo, z: i32) {
    roof_trusses(fb, p, g, z);
}

/// Scaffolding on the two visible faces up to `sh`: poles, ledgers, plank
/// decks, diagonal braces, and (optionally) green safety netting.
pub fn scaffold(fb: &mut Framebuffer, p: Proj, g: &Geo, sh: i32, left: bool, right: bool, net: bool) {
    let (x0, y0, x1, y1) = g.foot;
    let off = 0.2;
    let sh = sh.min(g.h + 12).max(12);
    let lift = 12;
    let sc = pal::SCAFFOLD;
    let sc_d = palette::shade(sc, 0.78);
    let mut face = |is_left: bool, lum: f32| {
        let (a0, a1) = if is_left { (x0 - off, x1 + off) } else { (y0 - off, y1 + off) };
        let wall_at = if is_left { y1 } else { x1 };
        let out_at = wall_at + off;
        let q = |a: f32, z: i32, out: bool| {
            let f = if out { out_at } else { wall_at };
            if is_left { p(a, f, z) } else { p(f, a, z) }
        };
        let n = ((a1 - a0) / 0.5).round().max(1.0) as usize;
        let at = |b: usize| a0 + (a1 - a0) * b as f32 / n as f32;
        let mut z = lift;
        while z <= sh {
            let deck = [q(at(0), z, false), q(at(n), z, false), q(at(n), z, true), q(at(0), z, true)];
            fb.fill_poly(&deck, palette::shade(pal::PLANK, lum));
            fb.line(q(at(0), z, true), q(at(n), z, true), palette::shade(sc, lum));
            z += lift;
        }
        for b in 0..=n {
            fb.line(q(at(b), 0, true), q(at(b), sh, true), palette::shade(sc, lum));
        }
        let mut z = 0;
        let mut k = 0;
        while z + lift <= sh {
            for b in 0..n {
                if (b + k) % 2 == 0 {
                    fb.line(q(at(b), z, true), q(at(b + 1), z + lift, true), palette::shade(sc_d, lum));
                }
            }
            z += lift;
            k += 1;
        }
        if net && sh > lift + 2 {
            let nq = [q(at(0), sh, true), q(at(n), sh, true), q(at(n), lift, true), q(at(0), lift, true)];
            fb.fill_poly_blended(&nq, palette::shade(pal::NETTING, lum), 0.42);
        }
    };
    if right {
        face(false, pal::FACE_RIGHT + 0.12);
    }
    if left {
        face(true, 1.0);
    }
    if left && right {
        fb.line(p(x1 + off, y1 + off, 0), p(x1 + off, y1 + off, sh), palette::shade(sc, 1.1));
    }
}

/// Stage 5: finishing. The building stands complete; the scaffold is half
/// down (the lit face only, up to the first floors), the netting gone.
pub fn finishing(fb: &mut Framebuffer, p: Proj, g: &Geo) {
    let sh = (g.h as f32 * 0.5) as i32;
    scaffold(fb, p, g, sh.max(12), true, false, false);
}

/// Everything a stage paints *before* the building (ground, fence behind).
pub fn before(fb: &mut Framebuffer, p: Proj, g: &Geo, stage: Stage) {
    ground(fb, p, g, stage);
    fence_back(fb, p, g, stage);
}

/// Everything painted after (the near fence runs).
pub fn after(fb: &mut Framebuffer, p: Proj, g: &Geo, stage: Stage) {
    fence_front(fb, p, g, stage);
}

/// A grid of window openings on the two near faces of a finished shell: one
/// row per storey, a pane every half tile, a few left dark.
pub fn glazing(fb: &mut Framebuffer, p: Proj, g: &Geo, glow: f32) {
    let (x0, y0, x1, y1) = g.foot;
    let lit = palette::mix(pal::WINDOW_DARK, pal::WINDOW_LIT, glow.clamp(0.0, 1.0));
    for (is_left, lum) in [(true, 1.0f32), (false, 0.8)] {
        let (a0, a1) = if is_left { (x0, x1) } else { (y0, y1) };
        let n = (((a1 - a0) / 0.5).round() as i32).max(1);
        let floors = (g.h / g.floor).max(1);
        for f in 0..floors {
            let z = f * g.floor + g.floor / 2 + 1;
            for k in 0..n {
                let a = a0 + (a1 - a0) * (k as f32 + 0.5) / n as f32;
                let (sx, sy) = if is_left { pt(p, a, y1, z) } else { pt(p, x1, a, z) };
                let on = hash(g.seed, (f * 31 + k) as u32 + if is_left { 0 } else { 777 }) < 0.8;
                let c = if on { palette::shade(lit, lum) } else { palette::shade(pal::WINDOW_DARK, lum * 0.8) };
                fb.fill_rect(sx - 1, sy - 2, 3, 3, c);
            }
        }
    }
}
