//! The moving geometry of the world's endings (B50), shared by the sim (who
//! runs from what) and the renderer (what is drawn where). Pure functions of
//! the ending's plan and its progress `u` (0 at the start of the warning, 1 at
//! the start of the spectacle, 2 at the aftermath, 3 at the morph).

fn ramp(x: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn hash(seed: u64, k: u32) -> f32 {
    let mut z = seed.wrapping_add(k as u64 * 0x9E37_79B9_7F4A_7C15).wrapping_add(0x1234_5678);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// What a giant is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GiantMode {
    Walk,
    Fight,
    Fall,
    Gone,
}

/// One of the striders.
#[derive(Clone, Copy, Debug)]
pub struct Giant {
    pub x: f32,
    pub y: f32,
    /// Facing, 0 = +x, 1 = +y, 2 = -x, 3 = -y (the nearest of the eight).
    pub dir: u8,
    /// Heading as a unit vector in tile space.
    pub head: (f32, f32),
    /// Distance walked, in tiles (drives the leg cycle).
    pub walked: f32,
    pub mode: GiantMode,
    /// 0..1 through the fall.
    pub fall: f32,
    pub which: u8,
}

fn dir_of(h: (f32, f32)) -> u8 {
    if h.0.abs() > h.1.abs() {
        if h.0 >= 0.0 { 0 } else { 2 }
    } else if h.1 >= 0.0 {
        1
    } else {
        3
    }
}

/// The striders at progress `u`. `aux` holds the walk: `a.x a.y b.x b.y two`.
pub fn giants(aux: &[f32; 8], u: f32) -> Vec<Giant> {
    let a = (aux[0], aux[1]);
    let b = (aux[2], aux[3]);
    let two = aux[4] > 0.5;
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1.0);
    let hd = ((b.0 - a.0) / len, (b.1 - a.1) / len);
    let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let lerp = |t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let mut out = Vec::new();
    if u < 0.82 {
        return out; // the warning: a shape on the hills at most (the sky draws it)
    }
    if !two {
        let q = ramp(u, 0.95, 1.98);
        let p = lerp(q);
        out.push(Giant {
            x: p.0,
            y: p.1,
            dir: dir_of(hd),
            head: hd,
            walked: q * len,
            mode: if q >= 1.0 { GiantMode::Gone } else { GiantMode::Walk },
            fall: 0.0,
            which: 0,
        });
        return out;
    }
    // Two: they meet at the middle, fight, one falls, the other walks on.
    let gap = 1.8;
    let m1 = (mid.0 - hd.0 * gap, mid.1 - hd.1 * gap);
    let m2 = (mid.0 + hd.0 * gap, mid.1 + hd.1 * gap);
    // Giant 0: a -> m1, fights, then on to b.
    let walk_in = ramp(u, 0.95, 1.4);
    let walk_out = ramp(u, 1.78, 1.98);
    let (p0, mode0, w0) = if u < 1.4 {
        let p = (a.0 + (m1.0 - a.0) * walk_in, a.1 + (m1.1 - a.1) * walk_in);
        (p, GiantMode::Walk, walk_in * dist(a, m1))
    } else if u < 1.78 {
        (m1, GiantMode::Fight, dist(a, m1))
    } else {
        let p = (m1.0 + (b.0 - m1.0) * walk_out, m1.1 + (b.1 - m1.1) * walk_out);
        (p, if walk_out >= 1.0 { GiantMode::Gone } else { GiantMode::Walk }, dist(a, m1) + walk_out * dist(m1, b))
    };
    out.push(Giant { x: p0.0, y: p0.1, dir: dir_of(hd), head: hd, walked: w0, mode: mode0, fall: 0.0, which: 0 });
    // Giant 1: b -> m2, fights, falls at 1.78..1.9, gone after.
    let nh = (-hd.0, -hd.1);
    let (p1, mode1, w1, fall) = if u < 1.4 {
        let p = (b.0 + (m2.0 - b.0) * walk_in, b.1 + (m2.1 - b.1) * walk_in);
        (p, GiantMode::Walk, walk_in * dist(b, m2), 0.0)
    } else if u < 1.78 {
        (m2, GiantMode::Fight, dist(b, m2), 0.0)
    } else if u < 1.95 {
        (m2, GiantMode::Fall, dist(b, m2), ramp(u, 1.78, 1.95))
    } else {
        (m2, GiantMode::Gone, dist(b, m2), 1.0)
    };
    out.push(Giant { x: p1.0, y: p1.1, dir: dir_of(nh), head: nh, walked: w1, mode: mode1, fall, which: 1 });
    out
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// A saucer: a ground position (tile space), its height above the ground in
/// pixels, and how hard its beam is on (0..1).
#[derive(Clone, Copy, Debug)]
pub struct Saucer {
    pub x: f32,
    pub y: f32,
    pub alt: f32,
    pub beam: f32,
    pub id: u8,
}

/// Where the fleet is at progress `u`; `secs` is real motion (wobble, drift).
pub fn saucers(site: (f32, f32), rad: f32, n: usize, seed: u64, u: f32, secs: f32) -> Vec<Saucer> {
    let mut out = Vec::new();
    if u < 0.55 || u >= 2.15 {
        return out;
    }
    for i in 0..n {
        let a = hash(seed, i as u32) * std::f32::consts::TAU;
        let ring = rad * (0.15 + 0.5 * hash(seed, 40 + i as u32));
        let hover = (site.0 + a.cos() * ring, site.1 + a.sin() * ring);
        // Come in from far out along the sky, at different times.
        let from = (hover.0 + 60.0 * (a + 0.8).cos(), hover.1 + 60.0 * (a + 0.8).sin());
        let delay = hash(seed, 80 + i as u32) * 0.12;
        let arrive = ease(ramp(u, 0.98 + delay, 1.22 + delay));
        let leave = ease(ramp(u, 1.86 + delay * 0.5, 2.08 + delay * 0.5));
        // Before the spectacle: a high, tiny light far over the east.
        let omen = ease(ramp(u, 0.55, 1.0));
        let (x, y, alt) = if u < 0.98 + delay {
            (from.0 * 0.4 + hover.0 * 0.6, from.1 * 0.4 + hover.1 * 0.6, 900.0 - 120.0 * omen)
        } else {
            let px = from.0 + (hover.0 - from.0) * arrive;
            let py = from.1 + (hover.1 - from.1) * arrive;
            let alt = 700.0 + (150.0 - 700.0) * arrive;
            let (px, py) = (px + (from.0 - hover.0) * leave * 0.7, py + (from.1 - hover.1) * leave * 0.7);
            (px, py, alt + 900.0 * leave)
        };
        let drift = (secs * 0.05 + i as f32 * 1.7).sin() * 0.35;
        let beam_on = ease(ramp(u, 1.25 + delay, 1.4 + delay)) * (1.0 - ease(ramp(u, 1.8, 1.9)));
        let pulse = 0.82 + 0.18 * (secs * 0.9 + i as f32 * 2.0).sin();
        out.push(Saucer { x: x + drift, y: y - drift * 0.6, alt, beam: beam_on * pulse, id: i as u8 });
    }
    out
}

/// A distant light high in the sky during the warning: a fleet far out. Screen
/// fractions of the sky band `(x, y, brightness)`.
pub fn far_lights(seed: u64, n: usize, u: f32) -> Vec<(f32, f32, f32)> {
    let mut v = Vec::new();
    if !(0.2..1.1).contains(&u) {
        return v;
    }
    for i in 0..n {
        let x = 0.55 + 0.4 * hash(seed, 200 + i as u32);
        let y = 0.12 + 0.45 * hash(seed, 220 + i as u32);
        let b = ease(ramp(u, 0.2 + 0.05 * i as f32, 0.7)) * (1.0 - ease(ramp(u, 1.0, 1.1)));
        v.push((x, y, b));
    }
    v
}
