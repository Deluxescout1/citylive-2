//! Spectacle art (B62, D75): everything here is original pixel art drawn from
//! code, in the same 1x vocabulary as the rest of the world (people 9x15,
//! tiles 64x32). The scene decides *what* is on and *where*; this module only
//! knows how to paint one of each thing into a framebuffer, clipped.
//!
//! Nothing allocates per frame except the small light list a fireworks show
//! fills, and nothing paints at all unless the scene asks.

use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::Camera;
use crate::palette::{self, Color, pal};
use crate::tex;

mod moving;
mod places;
mod square;
pub use moving::*;
pub use places::*;
pub use square::*;

/// What is on, as 0..1 progress through each event. The scene keeps a copy so
/// it can tell when something starts or stops.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Events {
    pub festival: Option<f32>,
    pub parade: Option<f32>,
    pub fireworks: Option<f32>,
    pub night_market: Option<f32>,
}

impl Events {
    /// The part that changes the *static* layer (stalls, stage, poles).
    pub fn dress(&self) -> Dress {
        Dress {
            festival: self.festival.is_some(),
            night_market: self.night_market.is_some(),
        }
    }
}

/// The dressing baked into the static layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dress {
    pub festival: bool,
    pub night_market: bool,
}

impl Dress {
    pub fn any(&self) -> bool {
        self.festival || self.night_market
    }
}

/// Multiply a colour by the scene's ambient tint, so a dynamic thing sits in
/// the same light as the baked world.
pub fn tinted(c: Color, tint: [f32; 3]) -> Color {
    palette::with_alpha(
        palette::rgb(
            clamp8(palette::r(c) as f32 * tint[0]),
            clamp8(palette::g(c) as f32 * tint[1]),
            clamp8(palette::b(c) as f32 * tint[2]),
        ),
        palette::alpha(c),
    )
}

fn clamp8(v: f32) -> u8 {
    v.clamp(0.0, 255.0) as u8
}

/// Add light to one pixel (saturating), the way a lamp adds to a dark street.
#[inline]
fn add_rgb(fb: &mut Framebuffer, x: i32, y: i32, rgb: [f32; 3]) {
    let c = fb.clip();
    if x < c.x0 || x >= c.x1 || y < c.y0 || y >= c.y1 {
        return;
    }
    let i = fb.index(x, y);
    let p = fb.px_mut();
    let d = p[i];
    p[i] = palette::with_alpha(
        palette::rgb(
            clamp8(palette::r(d) as f32 + rgb[0]),
            clamp8(palette::g(d) as f32 + rgb[1]),
            clamp8(palette::b(d) as f32 + rgb[2]),
        ),
        palette::alpha(d),
    );
}

// ---- lights ---------------------------------------------------------------

/// A short-lived point light (a firework's flash). Part of the B60 model's
/// artificial lights: added on top of the lit frame, so it only shows where the
/// scene is dark enough for it to.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub rgb: [f32; 3],
    pub k: f32,
}

impl Light {
    pub fn bounds(&self) -> Rect {
        Rect::new(
            (self.x - self.r) as i32,
            (self.y - self.r * 0.75) as i32,
            (self.x + self.r) as i32 + 1,
            (self.y + self.r * 0.75) as i32 + 1,
        )
    }
}

/// Paint the lights into the frame, within its clip. `dark` is how dark the
/// world is (0 = noon: invisible, 1 = night), so a daytime show adds almost
/// nothing and a night show lights the streets and walls it overlooks.
pub fn apply_lights(fb: &mut Framebuffer, lights: &[Light], dark: f32) {
    if dark <= 0.02 {
        return;
    }
    let clip = fb.clip();
    for l in lights {
        let b = l.bounds().intersect(&clip);
        if b.is_empty() || l.k <= 0.004 {
            continue;
        }
        let r2 = l.r * l.r;
        for y in b.y0..b.y1 {
            let dy = (y as f32 - l.y) / 0.75;
            for x in b.x0..b.x1 {
                let dx = x as f32 - l.x;
                let d2 = dx * dx + dy * dy;
                if d2 >= r2 {
                    continue;
                }
                let f = 1.0 - d2 / r2;
                // Soft radial falloff; light adds less to what is already
                // bright (a pale lot must not turn into a lavender slab) and
                // only a little to water (a glint, not a glowing surface).
                let i = fb.index(x, y);
                let d = fb.px()[i];
                let (dr, dg, db) = (palette::r(d) as f32, palette::g(d) as f32, palette::b(d) as f32);
                let luma = (0.3 * dr + 0.59 * dg + 0.11 * db) / 255.0;
                let mut a = f * f * f * l.k * dark * 48.0 * (1.0 - luma * 0.75);
                if db > dr + 25.0 && db > dg {
                    a *= 0.3;
                }
                add_rgb(fb, x, y, [l.rgb[0] * a, l.rgb[1] * a, l.rgb[2] * a]);
            }
        }
    }
}

// ---- fireworks --------------------------------------------------------------

/// Where a show goes up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Show {
    /// Screen x of the launch point and screen y of the ground there.
    pub cx: f32,
    pub ground_y: f32,
    /// Horizontal spread of the bursts, px.
    pub span: f32,
    /// True over water: the bursts are mirrored faintly in it.
    pub water: bool,
    /// The band of screen y the bursts open in (the sky, above the roofs).
    pub burst_top: f32,
    pub burst_bot: f32,
}

/// Seconds a show runs once begun.
pub const SHOW_SECS: f32 = 1200.0;
const PERIOD: f32 = 1.35;
const RISE: f32 = 0.9;
const LIFE: f32 = 3.2;

fn hsh(a: u32, b: u32) -> f32 {
    tex::hash1(a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA6B).rotate_left(7))
}

/// The burst numbers that can be airborne at `t`: the regular ones around `t`,
/// then the whole finale salvo (its bursts are 0.55 s apart, closer than the
/// regular `PERIOD`, so they cannot be found by dividing `t` by `PERIOD`).
fn burst_indices(t: f32, dur: f32) -> impl Iterator<Item = i32> {
    let main = ((dur * 0.88) / PERIOD) as i32;
    let first = ((t - RISE - LIFE) / PERIOD).floor() as i32 - 2;
    let last = ((t / PERIOD).ceil() as i32 + 1).min(main - 1);
    let finale = if t >= dur * 0.88 - 1.0 { main..main + 14 } else { 0..0 };
    (first.max(0)..=last).chain(finale)
}

/// Start time of burst `i`, or `None` past the end of the show. The last tenth
/// of the show is a finale: a quick salvo.
fn burst_start(i: i32, dur: f32, seed: u32) -> Option<f32> {
    let main = ((dur * 0.88) / PERIOD) as i32;
    if i < 0 {
        return None;
    }
    if i < main {
        Some(i as f32 * PERIOD + hsh(seed, i as u32) * 0.5)
    } else {
        let k = i - main;
        (k < 14).then(|| dur * 0.88 + k as f32 * 0.55 + hsh(seed, 77 + i as u32) * 0.2)
    }
}

const PALETTES: [(Color, Color); 7] = [
    (palette::rgb(0xff, 0x5a, 0x4a), palette::rgb(0xff, 0xc8, 0x60)),
    (palette::rgb(0x58, 0xa8, 0xff), palette::rgb(0xe8, 0xf4, 0xff)),
    (palette::rgb(0x6a, 0xff, 0x8a), palette::rgb(0xff, 0xf0, 0x80)),
    (palette::rgb(0xff, 0x70, 0xd0), palette::rgb(0xff, 0xd8, 0xf0)),
    (palette::rgb(0xff, 0xd0, 0x50), palette::rgb(0xff, 0x9a, 0x30)),
    (palette::rgb(0xb0, 0x80, 0xff), palette::rgb(0x70, 0xe0, 0xff)),
    (palette::rgb(0xff, 0xff, 0xff), palette::rgb(0xff, 0xe0, 0x90)),
];

/// The screen box a show can paint into (for damage).
pub fn show_bounds(s: &Show) -> Rect {
    let r = 190.0;
    Rect::new(
        (s.cx - s.span - r) as i32,
        (s.burst_top - r) as i32,
        (s.cx + s.span + r) as i32,
        (s.ground_y + if s.water { 60.0 } else { 20.0 }) as i32,
    )
}

const GRAV: [f32; 5] = [18.0, 16.0, 48.0, 16.0, 24.0];

/// The screen boxes the show paints into at `t`: a short box round each
/// climbing shell's head, a box round each opening burst (with the drop of its
/// falling sparks), and the circles its flashes light. Far smaller than
/// [`show_bounds`], so a frame of the show repaints a few hundred thousand
/// pixels, not a million.
pub fn show_rects(s: &Show, t: f32, dur: f32, seed: u32) -> Vec<Rect> {
    let mut out = Vec::new();
    for i in burst_indices(t, dur) {
        let Some(t0) = burst_start(i, dur, seed) else { continue };
        let age = t - t0;
        if age < 0.0 || age > RISE + LIFE {
            continue;
        }
        let h = seed ^ (i as u32).wrapping_mul(0x2545_F491);
        let lx = s.cx + (hsh(h, 1) - 0.5) * s.span * 0.5;
        let bx = s.cx + (hsh(h, 2) - 0.5) * s.span;
        let by = s.burst_top + hsh(h, 3) * (s.burst_bot - s.burst_top);
        let kind = (hsh(h, 5) * 5.0) as u32 % 5;
        let big = 115.0 + hsh(h, 6) * 85.0;
        if age < RISE {
            let u = age / RISE;
            let ee = |u: f32| 1.0 - (1.0 - u.max(0.0)) * (1.0 - u.max(0.0));
            let (e1, e0) = (ee(u), ee(u - 0.25));
            let (y1, y0) = (s.ground_y + (by - s.ground_y) * e1, s.ground_y + (by - s.ground_y) * e0);
            let x1 = lx + (bx - lx) * e1;
            let x0 = lx + (bx - lx) * e0;
            out.push(Rect::new(x0.min(x1) as i32 - 8, y1.min(y0) as i32 - 6, x0.max(x1) as i32 + 9, y1.max(y0) as i32 + 6));
            if u < 0.25 {
                out.push(Light { x: lx, y: s.ground_y - 6.0, r: 46.0, rgb: [0.0; 3], k: 1.0 }.bounds());
            }
            continue;
        }
        let a = age - RISE;
        let r = big * 1.05 + 8.0;
        let drop = GRAV[kind as usize] * a * a;
        let bottom = (by + r * 0.84 + drop + 8.0).min(s.ground_y + if s.water { 40.0 } else { 4.0 });
        out.push(Rect::new((bx - r) as i32, (by - r * 0.84 - 8.0) as i32, (bx + r) as i32 + 1, bottom as i32 + 1));
        // The flashes light for the first second or so.
        if a < 1.3 {
            out.push(Light { x: bx, y: s.ground_y - 120.0, r: big * 1.6, rgb: [0.0; 3], k: 1.0 }.bounds());
            out.push(Light { x: bx, y: by, r: big * 1.2, rgb: [0.0; 3], k: 1.0 }.bounds());
        }
    }
    out
}

/// Paint the show at `t` seconds in, pushing the lights it casts. Returns true
/// when anything is airborne.
pub fn draw_show(
    fb: &mut Framebuffer,
    s: &Show,
    t: f32,
    dur: f32,
    seed: u32,
    lights: &mut Vec<Light>,
) -> bool {
    let mut any = false;
    for i in burst_indices(t, dur) {
        let Some(t0) = burst_start(i, dur, seed) else { continue };
        let age = t - t0;
        if age < 0.0 || age > RISE + LIFE {
            continue;
        }
        any = true;
        let h = seed ^ (i as u32).wrapping_mul(0x2545_F491);
        // Launch near the middle, burst up and a little to the side.
        let lx = s.cx + (hsh(h, 1) - 0.5) * s.span * 0.5;
        let bx = s.cx + (hsh(h, 2) - 0.5) * s.span;
        let by = s.burst_top + hsh(h, 3) * (s.burst_bot - s.burst_top);
        let (c1, c2) = PALETTES[(hsh(h, 4) * PALETTES.len() as f32) as usize % PALETTES.len()];
        let kind = (hsh(h, 5) * 5.0) as u32 % 5;
        let big = 115.0 + hsh(h, 6) * 85.0;
        if age < RISE {
            // The shell climbs on a fading trail of gold sparks.
            let u = age / RISE;
            let e = 1.0 - (1.0 - u) * (1.0 - u);
            for k in 0..9 {
                let uu = (u - k as f32 * 0.025).max(0.0);
                let ee = 1.0 - (1.0 - uu) * (1.0 - uu);
                let x = lx + (bx - lx) * ee + ((k as f32 * 7.0 + age * 40.0).sin()) * 0.8;
                let y = s.ground_y + (by - s.ground_y) * ee;
                let a = (1.0 - k as f32 / 9.0) * 0.85;
                fb.put_blended(x as i32, y as i32, palette::rgb(0xff, 0xd8, 0x90), a);
                if k < 2 {
                    fb.put_blended(x as i32 + 1, y as i32, palette::rgb(0xff, 0xf0, 0xc0), a * 0.6);
                }
            }
            let _ = e;
            // A faint glow at the foot of the climb.
            if u < 0.25 {
                lights.push(Light {
                    x: lx,
                    y: s.ground_y - 6.0,
                    r: 46.0,
                    rgb: [1.0, 0.7, 0.35],
                    k: 0.35 * (1.0 - u * 4.0),
                });
            }
            continue;
        }
        let a = age - RISE;
        burst(fb, bx, by, big, a, kind, c1, c2, h, s.ground_y, s.water);
        // The flash lights what it overlooks: brightest at the pop, easing off.
        let k = (-a * 1.9).exp() * 0.95 + if a < 0.12 { 0.5 } else { 0.0 };
        // Two lights: the sky round the burst, and the ground it overlooks.
        lights.push(Light {
            x: bx,
            y: by,
            r: big * 1.2,
            rgb: [0.5, 0.5, 0.55],
            k: k * 0.5,
        });
        lights.push(Light {
            x: bx,
            y: s.ground_y - 120.0,
            r: big * 1.6,
            rgb: [
                palette::r(c1) as f32 / 255.0 * 0.8 + 0.25,
                palette::g(c1) as f32 / 255.0 * 0.8 + 0.25,
                palette::b(c1) as f32 / 255.0 * 0.8 + 0.25,
            ],
            k,
        });
    }
    any
}

#[allow(clippy::too_many_arguments)]
fn burst(
    fb: &mut Framebuffer,
    cx: f32,
    cy: f32,
    big: f32,
    a: f32,
    kind: u32,
    c1: Color,
    c2: Color,
    h: u32,
    ground_y: f32,
    water: bool,
) {
    // How far out a spark has flown after `a` seconds: fast, then easing.
    let reach = |a: f32| 1.0 - (-3.1 * a).exp();
    let (n, grav, tail, life) = match kind {
        0 => (96usize, 18.0f32, 6usize, 2.6f32), // peony
        1 => (84, 16.0, 8, 2.8),                  // chrysanthemum
        2 => (72, 48.0, 12, 3.0),                 // willow: gold, long falling trails
        3 => (60, 16.0, 5, 2.5),                  // ring
        _ => (56, 24.0, 8, 2.7),                  // palm: eight fat arms
    };
    let fade = |a: f32| (1.0 - a / life).clamp(0.0, 1.0);
    if a > life {
        return;
    }
    for i in 0..n {
        let ang = match kind {
            3 => i as f32 / n as f32 * std::f32::consts::TAU,
            4 => (i % 8) as f32 / 8.0 * std::f32::consts::TAU + (i / 8) as f32 * 0.035 - 0.08,
            _ => i as f32 / n as f32 * std::f32::consts::TAU + hsh(h, 300 + i as u32) * 0.4,
        };
        let sp = match kind {
            3 => 1.0,
            4 => 0.45 + (i / 8) as f32 * 0.13,
            _ => 0.62 + hsh(h, 400 + i as u32) * 0.46,
        };
        let col = if kind == 2 {
            palette::rgb(0xff, 0xcc, 0x58)
        } else if i % 3 == 0 {
            c2
        } else {
            c1
        };
        for k in 0..=tail {
            let aa = a - k as f32 * 0.04;
            if aa < 0.0 {
                break;
            }
            let r = big * sp * reach(aa);
            let x = cx + ang.cos() * r;
            // The sky is seen at a slant: bursts are slightly flattened, and
            // gravity drags every spark down over time.
            let y = cy + ang.sin() * r * 0.82 + grav * aa * aa;
            if y > ground_y {
                continue;
            }
            let f = fade(aa);
            let tailk = 1.0 - k as f32 / (tail as f32 + 1.0);
            // Hot white at the birth, the colour in the middle, an ember at the end.
            let hot = palette::mix(col, palette::rgb(0xff, 0xff, 0xf0), (1.0 - aa * 2.5).clamp(0.0, 1.0));
            let c = palette::mix(hot, palette::rgb(0xd0, 0x40, 0x20), ((aa - life * 0.55) / (life * 0.45)).clamp(0.0, 1.0));
            // Late sparks twinkle out.
            let tw = if aa > life * 0.6 && tex::hash2(i as i32, (aa * 18.0) as i32, h) < 0.4 { 0.2 } else { 1.0 };
            let cov = (f * tailk * tw).clamp(0.0, 1.0);
            let (ix, iy) = (x as i32, y as i32);
            fb.put_blended(ix, iy, c, (cov * 1.15).min(1.0));
            fb.put_blended(ix + 1, iy, c, (cov * 0.9).min(1.0));
            if k < 2 {
                // The head of a spark is 2x2 with a soft halo.
                fb.put_blended(ix, iy + 1, c, (cov * 0.9).min(1.0));
                fb.put_blended(ix + 1, iy + 1, c, (cov * 0.7).min(1.0));
                for (hx, hy) in [(-1, 0), (2, 0), (0, -1), (1, -1), (0, 2), (1, 2)] {
                    fb.put_blended(ix + hx, iy + hy, c, cov * 0.22);
                }
            }
            // Over water: a stretched, fainter mirror below the shore line.
            if water && k == 0 {
                let ry = ground_y + (ground_y - y) * 0.16;
                fb.put_blended(x as i32, ry as i32, c, cov * 0.22);
                fb.put_blended(x as i32, ry as i32 + 1, c, cov * 0.12);
            }
        }
    }
    // The pop: a bright disc that shrinks away.
    if a < 0.14 {
        let rr = 7.0 * (1.0 - a / 0.14);
        for dy in -8..=8 {
            for dx in -8..=8 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d <= rr {
                    fb.put_blended(cx as i32 + dx, cy as i32 + dy, palette::rgb(0xff, 0xf8, 0xe0), 1.0 - d / (rr + 1.0) * 0.4);
                }
            }
        }
    }
}

// ---- hot-air balloon ------------------------------------------------------

const SCHEMES: [[Color; 3]; 6] = [
    [palette::rgb(0xd8, 0x3a, 0x3a), palette::rgb(0xf4, 0xe8, 0xd0), palette::rgb(0x2a, 0x4a, 0x9a)],
    [palette::rgb(0xf0, 0xb0, 0x20), palette::rgb(0xc8, 0x30, 0x30), palette::rgb(0xf4, 0xe8, 0xd0)],
    [palette::rgb(0x28, 0x9a, 0x9a), palette::rgb(0xf4, 0xe8, 0xd0), palette::rgb(0xe0, 0x70, 0x20)],
    [palette::rgb(0x8a, 0x3a, 0xb0), palette::rgb(0xf0, 0xc8, 0x40), palette::rgb(0xf4, 0xe8, 0xd0)],
    [palette::rgb(0x3a, 0x9a, 0x4a), palette::rgb(0xf4, 0xe8, 0xd0), palette::rgb(0xd0, 0x40, 0x60)],
    [palette::rgb(0x2a, 0x6a, 0xc8), palette::rgb(0xf0, 0xd0, 0x40), palette::rgb(0xd8, 0x3a, 0x3a)],
];

/// A hot-air balloon with its basket's bottom at `(cx, base_y)`. `scale` is 1
/// for the common envelope (about 44 x 54 px, a 20 m balloon at the world's
/// scale), `burn` 0..1 is the burner's flame, `sag` 0..1 deflates the envelope
/// as it settles in a field. Returns its bounding box.
#[allow(clippy::too_many_arguments)]
pub fn draw_balloon(
    fb: &mut Framebuffer,
    cx: f32,
    base_y: f32,
    scale: f32,
    scheme: u8,
    burn: f32,
    sag: f32,
    t: f32,
    tint: [f32; 3],
) -> Rect {
    let col = SCHEMES[scheme as usize % SCHEMES.len()].map(|c| tinted(c, tint));
    let w = 58.0 * scale;
    let hgt = 72.0 * scale * (1.0 - sag * 0.62);
    let basket_h = 8.0 * scale;
    let rope = 9.0 * scale;
    let mouth_y = base_y - basket_h - rope;
    let top_y = mouth_y - hgt;
    let hw = w * 0.5 * (1.0 + sag * 0.35);
    let gores = 10.0;
    for yy in (top_y as i32)..=(mouth_y as i32) {
        let tt = ((yy as f32 - top_y) / hgt).clamp(0.0, 1.0);
        // Round shoulders at the top, widest at 40 %, tapering to the mouth.
        let half = if tt <= 0.42 {
            hw * (1.0 - ((0.42 - tt) / 0.42).powi(2)).max(0.0).sqrt()
        } else {
            hw * (1.0 - 0.66 * ((tt - 0.42) / 0.58).powf(1.35))
        };
        if half < 0.5 {
            continue;
        }
        let x0 = (cx - half).round() as i32;
        let x1 = (cx + half).round() as i32;
        for x in x0..=x1 {
            let u = ((x as f32 + 0.5 - cx) / half).clamp(-1.0, 1.0); // -1..1 across
            let ang = u.asin() / (std::f32::consts::FRAC_PI_2); // -1..1 round the barrel
            let gore = (((ang + 1.0) * 0.5 * gores) as i32).rem_euclid(gores as i32);
            let band = (0.56..0.66).contains(&tt);
            let mut c = if band {
                col[2]
            } else if gore % 2 == 0 {
                col[0]
            } else {
                col[1]
            };
            // Light from the upper left: a bright flank, a shaded one.
            let lit = 1.14 - (u + 1.0) * 0.30 - tt * 0.08;
            c = palette::shade(c, lit);
            if x == x0 || x == x1 {
                c = palette::shade(c, 0.7);
            }
            fb.put(x, yy, c);
        }
    }
    // The mouth: a short skirt in the third colour.
    let mh = (3.5 * scale) as i32;
    let mw = (hw * 0.34).max(2.0);
    for yy in 0..mh {
        for x in (cx - mw) as i32..=(cx + mw) as i32 {
            let c = if (x + yy) % 2 == 0 { col[2] } else { palette::shade(col[2], 0.82) };
            fb.put(x, mouth_y as i32 + yy, c);
        }
    }
    // Ropes and the basket.
    let rope_top = mouth_y + mh as f32;
    let basket_top = base_y - basket_h;
    let bw = 5.5 * scale;
    let rope_c = tinted(palette::rgb(0x4a, 0x3a, 0x2a), tint);
    fb.line((cx - mw, rope_top), (cx - bw, basket_top), rope_c);
    fb.line((cx + mw, rope_top), (cx + bw, basket_top), rope_c);
    for yy in 0..(basket_h as i32).max(3) {
        for x in (cx - bw) as i32..=(cx + bw) as i32 {
            let weave = (x + yy * 2) % 3 == 0;
            let c = if weave { palette::rgb(0x8a, 0x62, 0x3a) } else { palette::rgb(0xa8, 0x7e, 0x4c) };
            fb.put(x, basket_top as i32 + yy, tinted(if yy == 0 { palette::rgb(0x5a, 0x40, 0x28) } else { c }, tint));
        }
    }
    // The burner: a flame that flickers while she is climbing.
    if burn > 0.05 {
        let fl = (t * 23.0).sin() * 0.5 + 0.5;
        let fh = (4.0 + 5.0 * burn) * scale * (0.8 + 0.3 * fl);
        for k in 0..(fh as i32) {
            let f = k as f32 / fh;
            let c = palette::mix(palette::rgb(0xff, 0xf0, 0x90), palette::rgb(0xff, 0x70, 0x20), f);
            fb.put_blended(cx as i32, (rope_top - k as f32) as i32 + 2, c, 1.0 - f * 0.5);
            if k < (fh as i32) - 2 {
                fb.put_blended(cx as i32 + 1, (rope_top - k as f32) as i32 + 2, c, 0.6 - f * 0.4);
            }
        }
    }
    Rect::new(
        (cx - hw) as i32 - 2,
        top_y as i32 - 2,
        (cx + hw) as i32 + 3,
        base_y as i32 + 3,
    )
}

/// The balloon's shadow on the ground: a soft flat ellipse, offset by the sun
/// and shrinking as it rises (the shadow of something high is blurrier and
/// fainter). `strength` is the light's own direct strength, 0..1.
pub fn draw_balloon_shadow(fb: &mut Framebuffer, gx: f32, gy: f32, scale: f32, lift: f32, strength: f32) {
    if strength < 0.05 {
        return;
    }
    let rx = 22.0 * scale * (1.0 - lift * 0.15);
    let ry = rx * 0.45;
    let cov = (0.30 * strength * (1.0 - lift * 0.5)).clamp(0.0, 0.4);
    for y in (gy - ry) as i32..=(gy + ry) as i32 {
        for x in (gx - rx) as i32..=(gx + rx) as i32 {
            let dx = (x as f32 - gx) / rx;
            let dy = (y as f32 - gy) / ry;
            let d = dx * dx + dy * dy;
            if d < 1.0 {
                fb.put_blended(x, y, palette::rgb(0x10, 0x18, 0x10), cov * (1.0 - d * 0.5));
            }
        }
    }
}

// ---- rainbow ----------------------------------------------------------------

const BOW: [Color; 7] = [
    palette::rgb(0xe8, 0x40, 0x40),
    palette::rgb(0xf0, 0x90, 0x30),
    palette::rgb(0xf0, 0xe0, 0x40),
    palette::rgb(0x50, 0xc8, 0x50),
    palette::rgb(0x40, 0xc0, 0xe8),
    palette::rgb(0x40, 0x60, 0xe0),
    palette::rgb(0x90, 0x50, 0xd0),
];

/// A rainbow whose centre is `(cx, cy)` (below the horizon) and whose outer
/// edge has radius `r`, painted only above `floor_y`. `strength` 0..1. A
/// fainter, wider secondary bow with the colours reversed sits outside it when
/// the primary is strong.
pub fn draw_rainbow(fb: &mut Framebuffer, cx: f32, cy: f32, r: f32, strength: f32, floor_y: i32) {
    if strength <= 0.01 {
        return;
    }
    let band = 4.4f32;
    let mut one = |radius: f32, rev: bool, k: f32| {
        for (bi, base) in BOW.iter().enumerate() {
            let c = if rev { BOW[6 - bi] } else { *base };
            let rr = radius - bi as f32 * band;
            let x0 = (cx - rr) as i32;
            let x1 = (cx + rr) as i32;
            let mut prev_y: Option<i32> = None;
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let s = rr * rr - dx * dx;
                if s <= 0.0 {
                    continue;
                }
                let y = (cy - s.sqrt()) as i32;
                // The bow fades into the air at its feet and is brightest at the top.
                let across = (dx.abs() / rr).clamp(0.0, 1.0);
                let a = strength * k * (1.0 - across.powi(4) * 0.8) * 0.5;
                // Fill the run between this column's height and the last, so a
                // steep flank has no gaps.
                let (lo, hi) = match prev_y {
                    Some(p) => (y.min(p), y.max(p)),
                    None => (y, y),
                };
                for yy in lo..=hi {
                    if yy < floor_y {
                        fb.put_blended(x, yy, c, a);
                        fb.put_blended(x, yy + 1, c, a * 0.8);
                    }
                }
                prev_y = Some(y);
            }
        }
    };
    one(r, false, 1.0);
    if strength > 0.55 {
        one(r * 1.27, true, 0.38);
    }
}

// ---- lightning --------------------------------------------------------------

/// A bolt from `(x, y0)` down to `y1`: a white core with a blue glow, jagged
/// by seed. `k` 0..1 is the flash's current strength. A spectacular storm
/// forks. Returns the box it painted.
pub fn draw_bolt(fb: &mut Framebuffer, x: f32, y0: f32, y1: f32, seed: u32, spectacular: bool, k: f32) -> Rect {
    let mut pts: Vec<(f32, f32)> = vec![(x, y0)];
    let mut cx = x;
    let mut y = y0;
    let mut i = 0u32;
    while y < y1 {
        y += 9.0 + hsh(seed, i) * 13.0;
        cx += (hsh(seed, 50 + i) - 0.5) * 24.0;
        pts.push((cx, y.min(y1)));
        i += 1;
    }
    let mut bb = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    // The glow is a few soft translucent passes, the core a hard white line.
    for (dx, a) in [(-3.0f32, 0.10f32), (3.0, 0.10), (-2.0, 0.2), (2.0, 0.2), (-1.0, 0.45), (1.0, 0.45)] {
        for w in pts.windows(2) {
            let steps = ((w[1].1 - w[0].1).abs().max(1.0)) as i32;
            for s in 0..=steps {
                let f = s as f32 / steps as f32;
                let px = w[0].0 + (w[1].0 - w[0].0) * f + dx;
                let py = w[0].1 + (w[1].1 - w[0].1) * f;
                fb.put_blended(px as i32, py as i32, palette::rgb(0xa8, 0xb8, 0xff), a * k);
            }
        }
    }
    for w in pts.windows(2) {
        fb.line(w[0], w[1], palette::mix(palette::rgb(0xc8, 0xd4, 0xff), palette::rgb(0xff, 0xff, 0xff), k));
        for q in [w[0], w[1]] {
            bb = bb.union(&Rect::new(q.0 as i32 - 12, q.1 as i32 - 2, q.0 as i32 + 12, q.1 as i32 + 3));
        }
    }
    // Forks: a few short branches off the main stroke.
    let forks = if spectacular { 4 } else { 1 };
    for f in 0..forks {
        let at = 2 + (hsh(seed, 900 + f) * (pts.len().saturating_sub(4)) as f32) as usize;
        let Some(&(sx, sy)) = pts.get(at) else { continue };
        let dir = if hsh(seed, 910 + f) < 0.5 { -1.0 } else { 1.0 };
        let (mut bx, mut by) = (sx, sy);
        for s in 0..(3 + (hsh(seed, 920 + f) * 4.0) as i32) {
            let nx = bx + dir * (6.0 + hsh(seed, 930 + s as u32 + f * 7) * 9.0);
            let ny = by + 6.0 + hsh(seed, 940 + s as u32 + f * 7) * 9.0;
            if ny > y1 {
                break;
            }
            fb.line((bx, by), (nx, ny), palette::mix(palette::rgb(0xa8, 0xb8, 0xff), palette::rgb(0xf0, 0xf4, 0xff), k));
            bb = bb.union(&Rect::new(nx as i32 - 4, ny as i32 - 4, nx as i32 + 4, ny as i32 + 4));
            bx = nx;
            by = ny;
        }
    }
    bb
}

// ---- small creatures and props for the side monitors -------------------------

/// A running fox, about 16 x 9 px, facing `dir` (+1 right, -1 left); `frame`
/// alternates the legs. Feet at `(x, y)`.
pub fn draw_fox(fb: &mut Framebuffer, x: i32, y: i32, dir: i32, frame: u8, glow: f32, tint: [f32; 3]) {
    let fur = tinted(palette::rgb(0xd0, 0x6a, 0x24), tint);
    let dark = tinted(palette::rgb(0x8a, 0x40, 0x18), tint);
    let cream = tinted(palette::rgb(0xf0, 0xe0, 0xc8), tint);
    let mut p = |dx: i32, dy: i32, c: Color| fb.put(x + dx * dir, y + dy, c);
    // Body (long, low), chest, rump.
    for dx in -6..=3 {
        p(dx, -5, fur);
        p(dx, -4, fur);
    }
    for dx in -5..=2 {
        p(dx, -3, if dx > 0 { cream } else { fur });
    }
    p(-6, -4, dark);
    p(-5, -6, fur);
    p(-4, -6, fur);
    // Head, snout, ears.
    for dx in 4..=6 {
        p(dx, -6, fur);
        p(dx, -5, fur);
    }
    p(7, -5, dark);
    p(7, -6, fur);
    p(4, -7, dark);
    p(6, -7, dark);
    p(6, -6, tinted(palette::rgb(0x20, 0x18, 0x14), tint));
    // The brush: a long tail with a pale tip.
    for k in 0..5 {
        p(-7 - k, -5 + (k / 2), if k == 4 { cream } else { fur });
        p(-7 - k, -4 + (k / 2), fur);
    }
    // Legs, dark stockings, alternating.
    let (a, b) = if frame == 0 { (-4, 2) } else { (-2, 0) };
    for (lx, rise) in [(a, 0), (a + 1, 1), (b, 1), (b + 1, 0)] {
        p(lx, -2 + rise, dark);
        p(lx, -1 + rise, dark);
    }
    let _ = glow;
}

/// A kite: a diamond with a ribbon tail, centre `(x, y)`, tilted by `t`.
pub fn draw_kite(fb: &mut Framebuffer, x: f32, y: f32, t: f32, scheme: u8, tint: [f32; 3]) {
    let col = SCHEMES[scheme as usize % SCHEMES.len()].map(|c| tinted(c, tint));
    let sway = (t * 2.1).sin() * 2.0;
    let (cx, cy) = (x as i32, y as i32);
    for dy in -9..=9i32 {
        // A classic kite: broad shoulders above the cross, a long taper below.
        let half = if dy < 0 { 6 + dy * 6 / 10 } else { 6 - dy * 6 / 9 };
        let half = half.clamp(0, 6);
        for dx in -half..=half {
            let c = if dx == 0 || dy == 0 { col[1] } else if (dx < 0) == (dy < 0) { col[0] } else { col[2] };
            fb.put(cx + dx, cy + dy, c);
        }
    }
    // The tail wriggles away below.
    for k in 0..14 {
        let tx = cx as f32 + ((k as f32 * 0.7 + t * 4.0).sin()) * 2.0 + sway * (k as f32 / 14.0);
        let ty = cy as f32 + 10.0 + k as f32;
        let c = if k % 3 == 0 { col[0] } else { col[2] };
        fb.put(tx as i32, ty as i32, c);
    }
}

/// A few notes floating up from a street musician, `t` seconds.
pub fn draw_notes(fb: &mut Framebuffer, x: i32, y: i32, t: f32) {
    for n in 0..3 {
        let ph = (t * 0.45 + n as f32 * 0.33).fract();
        let nx = x as f32 + (ph * 9.0 + n as f32 * 3.0).sin() * 4.0 + n as f32 * 3.0;
        let ny = y as f32 - ph * 28.0;
        let a = (1.0 - ph).clamp(0.0, 1.0);
        let c = palette::rgb(0xf4, 0xe8, 0xc0);
        let (px, py) = (nx as i32, ny as i32);
        fb.put_blended(px, py, c, a);
        fb.put_blended(px, py - 1, c, a);
        fb.put_blended(px, py - 2, c, a);
        fb.put_blended(px - 1, py, c, a);
        fb.put_blended(px + 1, py - 2, c, a * 0.8);
    }
}

// ---- iso building blocks -----------------------------------------------------

/// A box on the ground from tile corners `(x0, y0)`-`(x1, y1)`, from height `z0`
/// to `z1` pixels above `base`. Faces shaded top / +y side / +x side.
#[allow(clippy::too_many_arguments)]
pub fn iso_box(
    fb: &mut Framebuffer,
    cam: &Camera,
    (x0, y0, x1, y1): (f32, f32, f32, f32),
    base: i32,
    z0: i32,
    z1: i32,
    top: Color,
) {
    let p = |x: f32, y: f32, z: i32| cam.project(x, y, base + z);
    // Left (+y) face and right (+x) face.
    let l = [p(x0, y1, z0), p(x1, y1, z0), p(x1, y1, z1), p(x0, y1, z1)];
    let r = [p(x1, y1, z0), p(x1, y0, z0), p(x1, y0, z1), p(x1, y1, z1)];
    let t = [p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)];
    fb.fill_poly(&l, palette::shade(top, 0.74));
    fb.fill_poly(&r, palette::shade(top, 0.58));
    fb.fill_poly(&t, top);
}

/// A string of pennants hung between two points (tile x, y, height px), with
/// a sag, in alternating colours. At 1x each flag is a 4-5 px triangle, which
/// reads as a dotted line of colour.
pub fn bunting(
    fb: &mut Framebuffer,
    cam: &Camera,
    a: (f32, f32, i32),
    b: (f32, f32, i32),
    cols: &[Color],
    sag: f32,
) {
    let pa = cam.project(a.0, a.1, a.2);
    let pb = cam.project(b.0, b.1, b.2);
    let len = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
    let n = (len / 5.0).max(2.0) as i32;
    let cord = palette::rgb(0x3a, 0x32, 0x2a);
    let mut last: Option<(f32, f32)> = None;
    for i in 0..=n {
        let f = i as f32 / n as f32;
        let x = pa.0 + (pb.0 - pa.0) * f;
        let y = pa.1 + (pb.1 - pa.1) * f + (f * (1.0 - f)) * 4.0 * sag;
        if let Some(l) = last {
            fb.line(l, (x, y), cord);
        }
        last = Some((x, y));
        if i % 1 == 0 && i < n {
            let c = cols[i as usize % cols.len()];
            let (px, py) = (x as i32, y as i32);
            fb.put(px, py + 1, c);
            fb.put(px + 1, py + 1, c);
            fb.put(px, py + 2, palette::shade(c, 0.85));
            fb.put(px + 1, py + 2, palette::shade(c, 0.85));
            fb.put(px, py + 3, palette::shade(c, 0.72));
        }
    }
}

/// A tent canopy (a 4-sided pyramid roof on poles), `(x, y)` its centre on
/// the ground, striped in two colours.
pub fn tent(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, a: Color, b: Color) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let p = |dx: f32, dy: f32, z: i32| cam.project(x + dx, y + dy, base + z);
    let (hx, hy) = (0.42, 0.42);
    let pole = palette::rgb(0xe8, 0xe8, 0xe8);
    // Back poles first, then the table, then the roof over the top.
    for (dx, dy) in [(-hx, -hy), (hx, -hy), (-hx, hy), (hx, hy)] {
        fb.line(p(dx, dy, 0), p(dx, dy, 13), pole);
    }
    iso_box(fb, cam, (x - 0.30, y - 0.1, x + 0.30, y + 0.28), base, 0, 5, palette::rgb(0xb8, 0x90, 0x60));
    let apex = p(0.0, 0.0, 19);
    let c = [p(-hx, -hy, 13), p(hx, -hy, 13), p(hx, hy, 13), p(-hx, hy, 13)];
    fb.fill_poly(&[apex, c[2], c[3]], palette::shade(a, 0.86));
    fb.fill_poly(&[apex, c[1], c[2]], palette::shade(b, 0.7));
    // Stripes on the visible faces.
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let l0 = (c[3].0 + (c[2].0 - c[3].0) * f, c[3].1 + (c[2].1 - c[3].1) * f);
        fb.line(apex, l0, if k % 2 == 0 { palette::shade(b, 0.9) } else { palette::shade(a, 0.7) });
    }
    fb.line(c[3], c[2], palette::shade(a, 0.6));
    fb.line(c[2], c[1], palette::shade(b, 0.55));
}

/// A grill with embers and a thread of smoke.
pub fn grill(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, t: f32) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (gx, gy) = cam.project(x, y, base);
    let (gx, gy) = (gx as i32, gy as i32);
    let steel = palette::rgb(0x2c, 0x2c, 0x34);
    fb.fill_rect(gx - 3, gy - 6, 7, 3, steel);
    fb.fill_rect(gx - 3, gy - 6, 7, 1, palette::rgb(0x56, 0x56, 0x60));
    fb.put(gx - 2, gy - 3, steel);
    fb.put(gx + 2, gy - 3, steel);
    fb.put(gx - 2, gy - 2, steel);
    fb.put(gx + 2, gy - 2, steel);
    fb.put(gx - 1, gy - 7, palette::rgb(0xff, 0x6a, 0x1a));
    fb.put(gx + 1, gy - 7, palette::rgb(0xff, 0x9a, 0x30));
    // Smoke wavers upward and thins.
    for k in 0..9 {
        let sx = gx as f32 + ((k as f32) * 0.8 + t * 1.6).sin() * (1.0 + k as f32 * 0.25);
        let sy = (gy - 9 - k) as f32;
        fb.put_blended(sx as i32, sy as i32, palette::rgb(0xb8, 0xb8, 0xc0), 0.55 - k as f32 * 0.055);
    }
}

/// A parked pickup truck (about 17 x 9 px), nose toward `dir` (+1 right).
pub fn pickup(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, dir: i32, body: Color) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (px, py) = cam.project(x, y, base);
    let (px, py) = (px as i32, py as i32);
    let mut q = |dx: i32, dy: i32, c: Color| fb.put(px + dx * dir, py + dy, c);
    let dark = palette::shade(body, 0.7);
    // Bed behind, cab in front.
    for dx in -8..=1 {
        q(dx, -3, if dx % 3 == 0 { dark } else { body });
        q(dx, -4, body);
    }
    for dx in 2..=7 {
        q(dx, -3, body);
        q(dx, -4, body);
        q(dx, -5, if dx < 6 { palette::rgb(0x8a, 0xb4, 0xd0) } else { body });
        q(dx, -6, if dx < 6 { palette::rgb(0x8a, 0xb4, 0xd0) } else { body });
    }
    for dx in -8..=7 {
        q(dx, -2, dark);
    }
    q(8, -3, palette::rgb(0xf0, 0xe8, 0xa0));
    for wx in [-5, 4] {
        q(wx, -1, pal::CAR_WHEEL);
        q(wx + 1, -1, pal::CAR_WHEEL);
        q(wx, 0, pal::CAR_WHEEL);
        q(wx + 1, 0, pal::CAR_WHEEL);
    }
}

/// A round lantern with a halo: `(x, y)` its centre. A lit one is emissive
/// (exact colour), so it survives the night tint.
pub fn lantern(fb: &mut Framebuffer, x: i32, y: i32, c: Color, lit: bool) {
    if lit {
        for dy in -3..=3i32 {
            for dx in -3..=3i32 {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if d > 1.5 && d < 3.6 {
                    fb.put_blended(x + dx, y + dy, c, 0.16 * (1.0 - (d - 1.5) / 2.1));
                }
            }
        }
        fb.put(x, y - 1, palette::mix(c, palette::rgb(0xff, 0xf0, 0xc0), 0.7));
        fb.put(x - 1, y, c);
        fb.put(x, y, palette::mix(c, palette::rgb(0xff, 0xf0, 0xc0), 0.45));
        fb.put(x + 1, y, c);
        fb.put(x, y + 1, palette::shade(c, 0.85));
    } else {
        fb.put(x, y - 1, palette::shade(c, 0.8));
        fb.put(x - 1, y, palette::shade(c, 0.9));
        fb.put(x, y, c);
        fb.put(x + 1, y, palette::shade(c, 0.7));
        fb.put(x, y + 1, palette::shade(c, 0.6));
    }
}

#[cfg(test)]
mod finale_tests {
    use super::*;

    /// Every burst that is airborne at `t` (by its own start time) must be one
    /// the draw and damage loops visit, finale salvo included.
    #[test]
    fn every_airborne_burst_is_visited_in_the_finale() {
        let dur = SHOW_SECS;
        for step in 0..((dur + 10.0) / 0.25) as i32 {
            let t = step as f32 * 0.25;
            let seen: Vec<i32> = burst_indices(t, dur).collect();
            for i in 0..1100 {
                let Some(t0) = burst_start(i, dur, 7) else { continue };
                let age = t - t0;
                if (0.0..=RISE + LIFE).contains(&age) {
                    assert!(seen.contains(&i), "burst {i} (age {age}) is airborne at t={t} but never drawn");
                }
            }
        }
    }
}
