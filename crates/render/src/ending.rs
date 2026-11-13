//! The world's endings, drawn (B50).
//!
//! The sim decides *what* is happening ([`Fx`] is its plain-data picture of
//! it); this module draws it: sky objects, ground overlays (lava, ice, the
//! machine traces, the flood's foam), creatures, saucers, smoke and fire.
//!
//! Everything is pixel art in the world's own style: hard-edged shapes with a
//! one-pixel dark outline, three or four tones per material, glows as
//! *dithered* stepped discs (never smooth gradients), and emissive things
//! (flames, crystals, beams, lights) written in their own bright colours so the
//! night grade does not dim them. Dull things (hulls, shells, smoke) take the
//! scene's ambient tint, so a saucer at dusk is dusk-coloured.
//!
//! Nothing here runs unless an ending is on: [`Scene`] holds an
//! `Option<Fx>`, and the layers below are only called while it is `Some`.

use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::Camera;
use crate::palette::{self, Color};
use map::{TileMap, endgeo};

// ---------------------------------------------------------------------------
// The picture the sim hands over
// ---------------------------------------------------------------------------

/// Which ending (the sim's `Ending::id()` order).
pub mod kind {
    pub const METEOR: u8 = 0;
    pub const VOLCANO: u8 = 1;
    pub const FLOOD: u8 = 2;
    pub const KAIJU: u8 = 3;
    pub const ALIENS: u8 = 4;
    pub const AI: u8 = 5;
    pub const RED_GIANT: u8 = 6;
    pub const ICE_AGE: u8 = 7;
    pub const MOON: u8 = 8;
    pub const DECLINE: u8 = 9;
    pub const SMOG: u8 = 10;
    pub const SUNSET: u8 = 11;
}

/// The phase codes (the sim's `Phase::id()`).
pub mod phase {
    pub const WARNING: u8 = 1;
    pub const SPECTACLE: u8 = 2;
    pub const AFTERMATH: u8 = 3;
    pub const MORPH: u8 = 4;
}

/// What the sim says is happening to the world.
#[derive(Clone, Debug, Default)]
pub struct Fx {
    pub kind: u8,
    pub phase: u8,
    /// Progress through the phase, 0..1.
    pub p: f32,
    /// Progress through the whole ending (warning 0..1, spectacle 1..2,
    /// aftermath 2..3, morph 3..4).
    pub u: f32,
    /// Seconds into the phase, and the phase's length.
    pub secs: f32,
    pub dur: f32,
    pub site: (f32, f32),
    pub aux: [f32; 8],
    pub seed: u64,
    pub frac: f32,
    /// Where the real sun is on the sky band, in pixels, if it is up.
    pub sun_px: Option<(f32, f32)>,
    /// Some lot is burning or smouldering (the scene sets it).
    pub fires: bool,
}

impl Fx {
    /// A fingerprint of the *slowly* changing picture (the sky, the grading,
    /// the lava/ice/trace/foam fronts): it changes a few times a minute at
    /// most, and a change recomposites the frame once. Everything that moves
    /// every frame is damage-limited instead ([`damage_rects`]).
    pub fn key(&self) -> u64 {
        let q = |v: f32, n: f32| (v * n) as i64 as u64;
        (self.kind as u64) << 56 ^ (self.phase as u64) << 48 ^ q(self.u, 160.0)
    }

    /// Is something here moving every frame (so the frame must be redrawn)?
    pub fn animated(&self) -> bool {
        (match self.kind {
            kind::METEOR | kind::MOON => self.u >= 1.0 && self.u < 3.0,
            kind::VOLCANO => self.u >= 0.1 && self.u < 3.0,
            kind::KAIJU => self.u >= 0.45 && self.u < 2.1,
            kind::ALIENS => self.u >= 0.9 && self.u < 2.2,
            kind::AI => self.u >= 1.0 && self.u < 2.6,
            kind::FLOOD => self.u >= 1.0 && self.u < 2.3,
            kind::SUNSET => self.u >= 1.1 && self.u < 2.6,
            _ => false,
        }) || self.fires
    }
}

/// Everything a layer needs besides the framebuffer.
pub struct Ctx<'a> {
    pub fx: &'a Fx,
    pub cam: &'a Camera,
    pub map: &'a TileMap,
    /// The overlay tiles (lava, ice, traces, foam): `(x, y, threshold)`.
    pub tiles: &'a [(i32, i32, f32)],
    /// Lots that are burning or smouldering: `(centre x, centre y, code, z px)`.
    pub marks: &'a [(f32, f32, u8, f32)],
    /// Converted residents, drawn as drones (tile coordinates).
    pub drones: &'a [(f32, f32)],
    pub depth: &'a [u16],
    /// The static layer, for its ground tags.
    pub stat: &'a [Color],
    pub tint: [f32; 3],
    /// 0 at noon .. 1 deep night.
    pub glow: f32,
    /// Wall-clock seconds, for motion that is not the sim's.
    pub t: f32,
    pub sky_h: i32,
    pub w: i32,
    pub h: i32,
}

// ---------------------------------------------------------------------------
// Small toolkit
// ---------------------------------------------------------------------------

#[inline]
fn hash(seed: u64, a: i32, b: i32) -> f32 {
    let mut z = seed
        ^ (a as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

#[inline]
fn ramp(x: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        return if x >= b { 1.0 } else { 0.0 };
    }
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}

#[inline]
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

const BAYER: [[f32; 4]; 4] = [
    [0.0, 8.0, 2.0, 10.0],
    [12.0, 4.0, 14.0, 6.0],
    [3.0, 11.0, 1.0, 9.0],
    [15.0, 7.0, 13.0, 5.0],
];

#[inline]
fn bayer(x: i32, y: i32) -> f32 {
    (BAYER[(y & 3) as usize][(x & 3) as usize] + 0.5) / 16.0
}

#[inline]
fn col(r: u8, g: u8, b: u8) -> Color {
    palette::rgb(r, g, b)
}

fn tinted(c: Color, tint: [f32; 3]) -> Color {
    col(
        (palette::r(c) as f32 * tint[0]).min(255.0) as u8,
        (palette::g(c) as f32 * tint[1]).min(255.0) as u8,
        (palette::b(c) as f32 * tint[2]).min(255.0) as u8,
    )
}

/// The scene's ambient tint, but never more than `keep` of the way down: foam,
/// white water and other things lit by their own event are drawn bright even at
/// night (the module's rule), where a plain tint would dim them into the ground.
fn tinted_floor(c: Color, tint: [f32; 3], keep: f32) -> Color {
    let k = keep.clamp(0.0, 1.0);
    let lift = |v: f32| v + (1.0 - v) * (1.0 - k);
    tinted(c, [lift(tint[0]), lift(tint[1]), lift(tint[2])])
}

/// Add light to a pixel (clipped, keeps the tag byte).
#[inline]
fn add(dst: &mut Framebuffer, x: i32, y: i32, c: (f32, f32, f32), k: f32) {
    let r = dst.clip();
    if x < r.x0 || x >= r.x1 || y < r.y0 || y >= r.y1 {
        return;
    }
    let i = dst.index(x, y);
    let p = dst.px()[i];
    let nr = (palette::r(p) as f32 + c.0 * k).min(255.0);
    let ng = (palette::g(p) as f32 + c.1 * k).min(255.0);
    let nb = (palette::b(p) as f32 + c.2 * k).min(255.0);
    dst.px_mut()[i] = palette::with_alpha(col(nr as u8, ng as u8, nb as u8), palette::alpha(p));
}

/// A stepped, dithered glow: concentric pixel-art rings, never a gradient.
fn glow(dst: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, c: (f32, f32, f32), strength: f32) {
    let r = dst.clip();
    let (x0, x1) = ((cx - rx) as i32, (cx + rx) as i32 + 1);
    let (y0, y1) = ((cy - ry) as i32, (cy + ry) as i32 + 1);
    let x0 = x0.max(r.x0);
    let x1 = x1.min(r.x1);
    let y0 = y0.max(r.y0);
    let y1 = y1.min(r.y1);
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                continue;
            }
            let lvl = (1.0 - d2).powf(1.4) * strength;
            // Four steps, dithered at the step edges.
            let steps = 4.0;
            let q = lvl * steps;
            let base = q.floor();
            let frac = q - base;
            let step = base + if frac > bayer(x, y) { 1.0 } else { 0.0 };
            if step > 0.0 {
                add(dst, x, y, c, step / steps * 0.55);
            }
        }
    }
}

/// Fill an ellipse with one colour at some coverage (dithered when `dither`).
fn ellipse(dst: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, c: Color, cov: f32, dither: bool) {
    let r = dst.clip();
    let x0 = ((cx - rx).floor() as i32).max(r.x0);
    let x1 = ((cx + rx).ceil() as i32 + 1).min(r.x1);
    let y0 = ((cy - ry).floor() as i32).max(r.y0);
    let y1 = ((cy + ry).ceil() as i32 + 1).min(r.y1);
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 + 0.5 - cx) / rx;
            let dy = (y as f32 + 0.5 - cy) / ry;
            if dx * dx + dy * dy <= 1.0 {
                if dither {
                    if cov > bayer(x, y) {
                        dst.put_blended(x, y, c, 1.0);
                    }
                } else {
                    dst.put_blended(x, y, c, cov);
                }
            }
        }
    }
}

/// A filled polygon with a one-pixel outline in a darker colour.
fn poly(dst: &mut Framebuffer, pts: &[(f32, f32)], fill: Color, outline: Color) {
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        let o: Vec<(f32, f32)> = pts.iter().map(|p| (p.0 + dx, p.1 + dy)).collect();
        dst.fill_poly(&o, outline);
    }
    dst.fill_poly(pts, fill);
}

/// A thick limb segment as a quad.
fn limb(a: (f32, f32), b: (f32, f32), wa: f32, wb: f32) -> [(f32, f32); 4] {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = (dx * dx + dy * dy).sqrt().max(0.001);
    let (nx, ny) = (-dy / l, dx / l);
    [
        (a.0 + nx * wa * 0.5, a.1 + ny * wa * 0.5),
        (b.0 + nx * wb * 0.5, b.1 + ny * wb * 0.5),
        (b.0 - nx * wb * 0.5, b.1 - ny * wb * 0.5),
        (a.0 - nx * wa * 0.5, a.1 - ny * wa * 0.5),
    ]
}

/// Is the pixel under `(x, y)` bare ground (or a lane) in the static layer?
#[inline]
fn ground_ok(c: &Ctx, x: i32, y: i32) -> bool {
    if x < 0 || y < 0 || x >= c.w || y >= c.h {
        return false;
    }
    let i = (y * c.w + x) as usize;
    let a = (c.stat[i] >> 24) as u8;
    a == 0xFF || (a == crate::framebuffer::TAG_BUILT && c.depth.get(i).map(|d| d & 1 == 1).unwrap_or(false))
}

/// The centre of a tile's top face, on screen.
#[inline]
fn tile_centre(c: &Ctx, x: i32, y: i32) -> (f32, f32) {
    let t = c.map.at(x, y);
    let lvl = (t.corner[0] as f32 + t.corner[1] as f32 + t.corner[2] as f32 + t.corner[3] as f32) * 0.25;
    c.cam.project(x as f32 + 0.5, y as f32 + 0.5, crate::iso::level_px(lvl))
}

/// Run `f(px, py, a, b)` over the pixels of a tile's top face, where `(a, b)`
/// are the tile-local coordinates in `-0.5..0.5`. Only ground pixels.
fn tile_pixels(c: &Ctx, dst: &mut Framebuffer, x: i32, y: i32, mut f: impl FnMut(&mut Framebuffer, i32, i32, f32, f32)) {
    let (cx, cy) = tile_centre(c, x, y);
    let clip = dst.clip();
    if (cx as i32) + 34 < clip.x0 || (cx as i32) - 34 >= clip.x1 || (cy as i32) + 18 < clip.y0 || (cy as i32) - 18 >= clip.y1 {
        return;
    }
    let (hw, hh) = (map::iso_consts::HALF_W as f32, map::iso_consts::HALF_H as f32);
    let y0 = (cy - hh).ceil() as i32;
    let y1 = (cy + hh).floor() as i32;
    for py in y0.max(clip.y0)..=y1.min(clip.y1 - 1) {
        let dy = py as f32 + 0.5 - cy;
        let half = hw * (1.0 - dy.abs() / hh);
        let xa = (cx - half).ceil() as i32;
        let xb = (cx + half).floor() as i32;
        for px in xa.max(clip.x0)..=xb.min(clip.x1 - 1) {
            if !ground_ok(c, px, py) {
                continue;
            }
            let dx = px as f32 + 0.5 - cx;
            let a = (dx / hw + dy / hh) * 0.5;
            let b = (dy / hh - dx / hw) * 0.5;
            f(dst, px, py, a, b);
        }
    }
}

fn ground_screen(c: &Ctx, x: f32, y: f32) -> (f32, f32) {
    let lvl = c.map.surface_level_at(x, y);
    c.cam.project(x, y, crate::iso::level_px(lvl))
}

// ---------------------------------------------------------------------------
// Layer 1: sky objects (under the clouds)
// ---------------------------------------------------------------------------

/// Things in the sky band: the swelling sun, the falling moon, the comet, the
/// far lights, the sky's wash. Drawn over the sky, under the clouds.
pub fn draw_sky(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let sky_h = c.sky_h;
    if sky_h <= 0 {
        return;
    }
    match fx.kind {
        kind::RED_GIANT => red_giant(dst, c),
        kind::MOON => doom_moon(dst, c),
        kind::METEOR => comet(dst, c),
        kind::ALIENS => far_fleet(dst, c),
        kind::SMOG => sky_wash(dst, c, (0x7a, 0x62, 0x44), 0.72 * ramp(fx.u, 0.1, 1.3)),
        kind::SUNSET => sunset_sky(dst, c),
        kind::VOLCANO => {
            let k = 0.5 * ramp(fx.u, 1.0, 1.4) * (1.0 - 0.6 * ramp(fx.u, 2.0, 3.0));
            sky_wash(dst, c, (0x3a, 0x30, 0x2e), k);
        }
        kind::ICE_AGE => sky_wash(dst, c, (0xb8, 0xc8, 0xd8), 0.45 * ramp(fx.u, 0.4, 1.4)),
        kind::AI => sky_wash(dst, c, (0x16, 0x30, 0x46), 0.35 * ramp(fx.u, 1.0, 1.8) * (1.0 - ramp(fx.u, 2.4, 3.0))),
        _ => {}
    }
}

/// Tint the sky band toward a colour, stronger toward the horizon.
fn sky_wash(dst: &mut Framebuffer, c: &Ctx, tone: (u8, u8, u8), k: f32) {
    if k <= 0.01 {
        return;
    }
    let clip = dst.clip();
    let target = col(tone.0, tone.1, tone.2);
    for y in clip.y0.max(0)..clip.y1.min(c.sky_h) {
        let f = 0.45 + 0.55 * (y as f32 / c.sky_h as f32);
        let cov = (k * f).clamp(0.0, 1.0);
        for x in clip.x0..clip.x1 {
            let i = dst.index(x, y);
            let p = dst.px()[i];
            dst.px_mut()[i] = palette::blend(p, target, cov);
        }
    }
}

fn red_giant(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let g = ease(ramp(fx.u, 0.0, 1.6));
    sky_wash(dst, c, (0xb0, 0x30, 0x14), 0.62 * ease(ramp(fx.u, 0.15, 1.5)));
    let (cx, cy) = fx.sun_px.unwrap_or((c.w as f32 * 0.62, c.sky_h as f32 * 0.42));
    // Grows from the sun's size to most of the sky.
    let r = 16.0 + g * g * c.sky_h as f32 * 0.95;
    let heat = ease(ramp(fx.u, 0.0, 1.2));
    let r2 = r * r;
    let clip = dst.clip();
    let (x0, x1) = (((cx - r - 14.0) as i32).max(clip.x0), ((cx + r + 14.0) as i32 + 1).min(clip.x1));
    let (y0, y1) = (((cy - r - 14.0) as i32).max(clip.y0), ((cy + r + 14.0) as i32 + 1).min(clip.y1.min(c.sky_h)));
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 {
                let d = (d2 / r2).sqrt();
                // Granulation: big slow cells, a darker limb.
                let cell = hash(fx.seed, (x as f32 / 7.0 + c.t * 0.05) as i32, (y as f32 / 7.0) as i32);
                let tone = (1.0 - d * d * 0.8) + (cell - 0.5) * 0.28;
                let heat_tone = tone * (0.6 + 0.4 * heat);
                let colr = if heat_tone > 0.82 {
                    col(0xff, 0xe8, 0xa8)
                } else if heat_tone > 0.62 {
                    col(0xff, 0xa8, 0x38)
                } else if heat_tone > 0.42 {
                    col(0xe4, 0x5a, 0x1e)
                } else if heat_tone > 0.24 {
                    col(0xb4, 0x2c, 0x16)
                } else {
                    col(0x7c, 0x1a, 0x12)
                };
                dst.put_blended(x, y, colr, 1.0);
            } else if d2 <= (r + 12.0) * (r + 12.0) {
                // The corona: a dithered ring.
                let d = (d2.sqrt() - r) / 12.0;
                if (1.0 - d) * 0.8 > bayer(x, y) {
                    dst.put_blended(x, y, col(0xff, 0x7a, 0x2a), 0.85);
                }
            }
        }
    }
    // A wide red glow so the whole band feels the heat.
    // (The wide red glow is the sky wash above: a glow disc this size cost more
    // than the rest of the bake together.)
    glow(dst, cx, cy, r * 1.25 + 14.0, r * 1.25 + 14.0, (255.0, 90.0, 30.0), 0.8 * g);
}

fn doom_moon(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    // Grows over days; in the spectacle it swells to fill the band and drops.
    let g = ease(ramp(fx.u, 0.0, 1.0));
    let fall = ease(ramp(fx.u, 1.0, 1.7));
    let r = c.sky_h as f32 * (0.07 + 0.38 * g * g + 1.35 * fall * fall);
    let cx = c.w as f32 * (0.30 - 0.04 * fall);
    let cy = c.sky_h as f32 * (0.36 + 0.5 * fall);
    if fx.u >= 1.74 {
        return;
    }
    let clip = dst.clip();
    let (x0, x1) = (((cx - r) as i32).max(clip.x0), ((cx + r) as i32 + 1).min(clip.x1));
    let (y0, y1) = (((cy - r) as i32).max(clip.y0), ((cy + r) as i32 + 1).min(clip.y1.min(c.sky_h)));
    let night = 0.55 + 0.45 * c.glow;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = (x as f32 + 0.5 - cx) / r;
            let dy = (y as f32 + 0.5 - cy) / r;
            let d2 = dx * dx + dy * dy;
            if d2 > 1.0 {
                continue;
            }
            // Lit from the upper left, a soft terminator on the lower right.
            let lit = 0.45 + 0.55 * (1.0 - (dx * 0.55 + dy * 0.5 + 0.25).clamp(0.0, 1.0)) * (1.0 - d2 * 0.3);
            // Maria: broad dark seas from low-frequency noise (not hash blocks).
            let sea = map::rng::fbm(dx * 2.6 + 7.0, dy * 2.6 + 3.0, fx.seed ^ 0x77, 3);
            let mut v = lit * if sea < 0.42 { 0.74 } else { 1.0 };
            // Craters: bright rims with darker floors.
            for k in 0..14 {
                let (qx, qy) = (hash(fx.seed, k, 1) * 1.7 - 0.85, hash(fx.seed, k, 2) * 1.7 - 0.85);
                let qr = 0.035 + hash(fx.seed, k, 3) * 0.09;
                let d = ((dx - qx).powi(2) + (dy - qy).powi(2)).sqrt();
                let px = (r.max(40.0) / 40.0).max(1.0);
                if (d - qr).abs() < 0.012 * px.min(2.5) {
                    v *= 1.12;
                } else if d < qr {
                    v *= 0.88;
                }
            }
            let v = v * night;
            // Banded to a handful of tones, as the rest of the art is.
            let q = (v * 6.0).round() / 6.0;
            let colr = col((q * 236.0) as u8, (q * 232.0) as u8, (q * 218.0 + 4.0) as u8);
            dst.put_blended(x, y, colr, 1.0);
        }
    }
    // Rim light.
    glow(dst, cx, cy, r * 1.25, r * 1.25, (200.0, 200.0, 190.0), 0.45 * g);
    // The fall: a hot red edge and falling embers.
    if fall > 0.0 {
        glow(dst, cx, cy + r * 0.2, r * 1.15, r * 1.1, (255.0, 120.0, 40.0), 0.8 * fall);
    }
}

fn comet(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    if fx.u >= 1.0 + 0.36 || fx.u < 0.02 {
        return;
    }
    // Before the spectacle the comet hangs over the east, growing; during it,
    // it is the fireball (drawn in the world layer).
    if fx.u >= 1.0 {
        return;
    }
    let g = ease(ramp(fx.u, 0.02, 1.0));
    let hx = c.w as f32 * (0.22 + 0.04 * g);
    let hy = c.sky_h as f32 * (0.26 + 0.18 * g);
    let tail = 24.0 + 130.0 * g;
    let (tx, ty) = (0.86f32, -0.5f32); // the tail points away from the sun
    let n = tail as i32;
    for i in (0..n).rev() {
        let f = i as f32 / tail;
        let w = 1.0 + 3.5 * g * (1.0 - f) + f * 2.0;
        let px = hx + tx * i as f32;
        let py = hy + ty * i as f32;
        let a = (1.0 - f).powf(1.4);
        let cc = if f < 0.2 { col(0xff, 0xf4, 0xd8) } else if f < 0.55 { col(0xb8, 0xe4, 0xff) } else { col(0x6a, 0x9a, 0xe0) };
        for k in -(w as i32)..=(w as i32) {
            let dither = a * (1.0 - (k as f32 / (w + 0.5)).abs() * 0.7);
            let (x, y) = ((px + k as f32 * 0.5) as i32, (py + k as f32 * 0.86) as i32);
            if dither > bayer(x, y) {
                dst.put_blended(x, y, cc, 0.95);
            }
        }
    }
    let hr = 2.0 + 3.5 * g;
    glow(dst, hx, hy, hr * 5.0, hr * 5.0, (255.0, 230.0, 180.0), 0.8);
    ellipse(dst, hx, hy, hr, hr, col(0xff, 0xff, 0xf0), 1.0, false);
    ellipse(dst, hx - 0.5, hy - 0.5, hr * 0.5, hr * 0.5, col(0xff, 0xff, 0xff), 1.0, false);
}

fn far_fleet(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let n = fx.aux[2].max(3.0) as usize;
    for (fxp, fyp, b) in endgeo::far_lights(fx.seed, n + 2, fx.u) {
        if b <= 0.02 {
            continue;
        }
        let x = fxp * c.w as f32;
        let y = fyp * c.sky_h as f32;
        glow(dst, x, y, 9.0, 6.0, (200.0, 240.0, 255.0), 0.9 * b);
        let tw = 0.6 + 0.4 * (c.t * 1.7 + x * 0.1).sin();
        dst.put_blended(x as i32, y as i32, col(0xf0, 0xff, 0xff), b * tw);
        dst.put_blended(x as i32 - 1, y as i32, col(0xc8, 0xf0, 0xff), b * 0.7);
        dst.put_blended(x as i32 + 1, y as i32, col(0xc8, 0xf0, 0xff), b * 0.7);
    }
}

fn sunset_sky(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    // Gold in the warning, dusk violet through the spectacle, a pink dawn after.
    let gold = ramp(fx.u, 0.1, 0.9) * (1.0 - ramp(fx.u, 1.3, 1.7));
    let dusk = ramp(fx.u, 1.1, 1.7) * (1.0 - ramp(fx.u, 2.0, 2.4));
    let dawn = ramp(fx.u, 2.0, 2.4) * (1.0 - ramp(fx.u, 3.0, 3.6));
    sky_wash(dst, c, (0xff, 0xb0, 0x4a), 0.5 * gold);
    sky_wash(dst, c, (0x2a, 0x1c, 0x4c), 0.75 * dusk);
    sky_wash(dst, c, (0xff, 0x9a, 0x92), 0.5 * dawn);
    // The slow sun on the horizon in the dawn.
    if dawn > 0.1 {
        let (cx, cy) = (c.w as f32 * 0.5, c.sky_h as f32 * (0.98 - 0.12 * dawn));
        glow(dst, cx, cy, 160.0, 90.0, (255.0, 200.0, 140.0), 0.9 * dawn);
        ellipse(dst, cx, cy, 34.0, 34.0, col(0xff, 0xe8, 0xb0), 1.0, false);
        ellipse(dst, cx, cy, 24.0, 24.0, col(0xff, 0xf6, 0xd8), 1.0, false);
    }
    if gold > 0.1 {
        // Long rays fanning out from the low sun, in dithered bands.
        let (sx0, sy0) = (c.w as f32 * 0.72, c.sky_h as f32 * (0.7 + 0.28 * ramp(fx.u, 0.5, 1.3)));
        for ray in 0..9 {
            let a = -2.9 + ray as f32 * 0.32 + hash(fx.seed, ray, 3) * 0.1;
            for i in 0..(c.w as i32 / 2) {
                let (x, y) = (sx0 + a.cos() * i as f32 * 1.4, sy0 + a.sin() * i as f32 * 0.5);
                if y < 0.0 || y >= c.sky_h as f32 || x < 0.0 || x >= c.w as f32 {
                    continue;
                }
                let f = 1.0 - i as f32 / (c.w as f32 * 0.5);
                for w in 0..3 {
                    if f * 0.5 * gold > bayer(x as i32 + w, y as i32) {
                        dst.put_blended(x as i32 + w, y as i32, col(0xff, 0xd8, 0x90), 0.5 * gold);
                    }
                }
            }
        }
        let (cx, cy) = (c.w as f32 * 0.72, c.sky_h as f32 * (0.7 + 0.28 * ramp(fx.u, 0.5, 1.3)));
        glow(dst, cx, cy, 190.0, 110.0, (255.0, 170.0, 70.0), 0.8 * gold);
    }
}

// ---------------------------------------------------------------------------
// Layer 2: the ground (under the buildings, over the tiles)
// ---------------------------------------------------------------------------

pub fn draw_ground(dst: &mut Framebuffer, c: &Ctx) {
    match c.fx.kind {
        kind::VOLCANO => lava(dst, c),
        kind::ICE_AGE => ice(dst, c),
        kind::AI => traces(dst, c),
        kind::FLOOD => foam(dst, c),
        kind::SMOG => haze_bands(dst, c),
        kind::SUNSET => golden_land(dst, c),
        _ => {}
    }
}

/// Horizontal bands of brown haze lying over the land, thicker toward the
/// horizon, thickening with the smog. Static.
fn haze_bands(dst: &mut Framebuffer, c: &Ctx) {
    let k = ramp(c.fx.u, 0.2, 1.3) * (1.0 - 0.7 * ramp(c.fx.u, 2.8, 4.0));
    if k <= 0.02 {
        return;
    }
    let clip = dst.clip();
    let (y0, y1) = (clip.y0.max(c.sky_h), clip.y1);
    let mut y = c.sky_h + 20;
    let mut band = 0;
    while y < y1 {
        let th = 30 + (hash(c.fx.seed, band, 5) * 40.0) as i32;
        let a = (0.50 - (y - c.sky_h) as f32 / c.h as f32 * 0.2).max(0.2) * k;
        for yy in y.max(y0)..(y + th).min(y1) {
            let edge = ((yy - y) as f32 / th as f32 - 0.5).abs() * 2.0;
            let f = (1.0 - edge).powf(1.3);
            for x in clip.x0..clip.x1 {
                dst.put_blended(x, yy, col(0xa8, 0x8a, 0x58), (a * 0.7 * f).min(0.6));
            }
        }
        y += 80 + (hash(c.fx.seed, band, 6) * 70.0) as i32;
        band += 1;
    }
}

/// A long, low golden light over the land.
fn golden_land(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let gold = ramp(fx.u, 0.15, 0.9) * (1.0 - ramp(fx.u, 1.2, 1.7));
    if gold <= 0.02 {
        return;
    }
    let clip = dst.clip();
    let y0 = clip.y0.max(c.sky_h);
    for y in y0..clip.y1 {
        if (y & 1) == 0 {
            continue;
        }
        for x in (clip.x0..clip.x1).step_by(2) {
            if bayer(x, y) < 0.5 {
                dst.put_blended(x, y, col(0xff, 0xb4, 0x50), 0.16 * gold);
            }
        }
    }
}

/// The cone: dark basalt over the slope (darkest at the summit), a glowing
/// crater and thin lava runs down the sides. Static between slow ticks.
fn vent_screen(c: &Ctx) -> (f32, f32) {
    let fx = c.fx;
    if fx.aux[6] > 0.5 {
        c.cam.project(fx.site.0, fx.site.1, crate::iso::level_px(fx.aux[7]))
    } else {
        ground_screen(c, fx.site.0, fx.site.1)
    }
}

fn cone(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let build = ramp(fx.u, 0.15, 0.95);
    if build <= 0.02 {
        return;
    }
    let r_cone = 10.0f32;
    let (sx, sy) = fx.site;
    let hot = ramp(fx.u, 0.5, 1.1) * (1.0 - 0.7 * ramp(fx.u, 2.0, 3.0));
    let massif = fx.aux[6] > 0.5;
    for ty in (sy as i32 - 10)..=(sy as i32 + 10) {
        for tx in (sx as i32 - 10)..=(sx as i32 + 10) {
            if massif {
                break;
            }
            let d = ((tx as f32 + 0.5 - sx).powi(2) + (ty as f32 + 0.5 - sy).powi(2)).sqrt();
            if d >= r_cone || !c.map.in_bounds(tx, ty) || c.map.morphed(tx, ty) {
                continue;
            }
            let k = (1.0 - d / r_cone).powf(0.8);
            let a = (0.18 + 0.62 * k) * build;
            tile_pixels(c, dst, tx, ty, |dst, px, py, _a, _b| {
                let v = hash(fx.seed, px >> 1, py >> 1);
                let tone = if v < 0.4 { col(0x1c, 0x18, 0x1a) } else { col(0x2c, 0x26, 0x26) };
                dst.put_blended(px, py, tinted(tone, c.tint), a);
            });
        }
    }
    // Lava runs: thin glowing veins from the crater down three sides.
    let crater = vent_screen(c);
    for run in 0..4 {
        let ang = hash(fx.seed, run, 91) * std::f32::consts::TAU;
        let len = (3.0 + 5.5 * hot * (0.6 + 0.4 * hash(fx.seed, run, 92))).min(9.0);
        let mut s_t = 1.2f32;
        while s_t < len {
            let wob = (s_t * 1.7 + run as f32 * 2.0).sin() * 0.35;
            let (px, py) = (sx + ang.cos() * s_t - ang.sin() * wob, sy + ang.sin() * s_t + ang.cos() * wob);
            let g = if massif { (crater.0 + (px - sx) * 22.0 - (py - sy) * 22.0, crater.1 + (px - sx + py - sy) * 11.0) } else { ground_screen(c, px, py) };
            let f = 1.0 - s_t / len;
            let colr = if f > 0.6 { col(0xff, 0xe0, 0x78) } else if f > 0.25 { col(0xff, 0x90, 0x28) } else { col(0xd0, 0x40, 0x14) };
            dst.fill_rect(g.0 as i32, g.1 as i32 - 1, 2, 2, colr);
            if (s_t * 4.0) as i32 % 3 == 0 {
                glow(dst, g.0, g.1, 9.0, 6.0, (255.0, 110.0, 30.0), 0.55 * hot);
            }
            s_t += 0.25;
        }
    }
    // The crater itself: a hot mouth with a halo, visible by day.
    let mouth = (crater.0, crater.1 - 2.0);
    ellipse(dst, mouth.0, mouth.1, 8.0 * (0.5 + 0.5 * build), 4.0 * (0.5 + 0.5 * build), col(0x1a, 0x10, 0x10), 1.0, false);
    ellipse(dst, mouth.0, mouth.1, 6.0 * hot, 3.0 * hot, col(0xff, 0x8a, 0x20), 1.0, false);
    ellipse(dst, mouth.0, mouth.1, 3.0 * hot, 1.5 * hot, col(0xff, 0xf0, 0xa0), 1.0, false);
    glow(dst, mouth.0, mouth.1, 46.0, 28.0, (255.0, 100.0, 24.0), 0.8 * hot);
}

fn lava(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    cone(dst, c);
    if fx.u < 1.08 {
        return;
    }
    let frac = fx.frac;
    // Heat: hot while it runs, cooling through the aftermath.
    let heat_all = if fx.u < 2.0 { 1.0 } else { (1.0 - ramp(fx.u, 2.0, 3.0)).powf(0.8) };
    let t = c.t;
    let seed = fx.seed;
    for &(x, y, thr) in c.tiles {
        if thr > frac || c.map.morphed(x, y) {
            continue;
        }
        let age = (frac - thr).max(0.0);
        // The front is white-hot; the old flow has a crust with glowing seams.
        let heat = (heat_all * (1.0 - (age * 2.2).min(0.45))).clamp(0.0, 1.0);
        let front = if fx.u < 2.0 { 1.0 - ramp(age, 0.0, 0.10) } else { 0.0 };
        tile_pixels(c, dst, x, y, |dst, px, py, a, b| {
            let cx = ((a + 0.5) * 8.0 + (x * 3) as f32) as i32;
            let cy = ((b + 0.5) * 8.0 + (y * 5) as f32) as i32;
            let cell = hash(seed ^ 0x1A7A, cx, cy);
            let flow = 0.5 + 0.5 * (fx.u * 600.0 + cell * 6.28).sin();
            let seam = cell < 0.34 + 0.2 * heat;
            let colr = if seam && heat > 0.12 {
                let h = heat * (0.6 + 0.4 * flow) + front * 0.4;
                if h > 0.85 {
                    col(0xff, 0xf0, 0x9a)
                } else if h > 0.6 {
                    col(0xff, 0xb4, 0x2c)
                } else if h > 0.38 {
                    col(0xf0, 0x6a, 0x1a)
                } else {
                    col(0xa8, 0x2c, 0x14)
                }
            } else {
                let v = hash(seed, px >> 1, py >> 1);
                if v < 0.35 {
                    col(0x1c, 0x18, 0x18)
                } else if v < 0.75 {
                    col(0x2a, 0x24, 0x22)
                } else {
                    col(0x3a, 0x32, 0x2e)
                }
            };
            let lit = seam && heat > 0.12;
            let colr = if lit { colr } else { tinted(colr, c.tint) };
            dst.put_blended(px, py, colr, 1.0);
        });
    }
    // A glow over the hottest tiles: at night this is the whole picture.
    if heat_all > 0.15 {
        let mut n = 0;
        for &(x, y, thr) in c.tiles {
            if thr > frac || c.map.morphed(x, y) {
                continue;
            }
            n += 1;
            if n % 5 != 0 {
                continue;
            }
            let (cx, cy) = tile_centre(c, x, y);
            glow(dst, cx, cy - 2.0, 34.0, 20.0, (255.0, 110.0, 30.0), 0.55 * heat_all * (0.4 + 0.6 * c.glow));
        }
    }
}

fn ice(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let frac = ramp(fx.u, 0.3, 1.4);
    if frac <= 0.0 {
        return;
    }
    for &(x, y, thr) in c.tiles {
        if thr > frac || c.map.morphed(x, y) {
            continue;
        }
        let melt = ramp(fx.u, 3.0, 3.8);
        let seed = fx.seed;
        tile_pixels(c, dst, x, y, |dst, px, py, a, b| {
            let v = hash(seed, px >> 1, py >> 1);
            // Cracks: thin lines along a few tile-local diagonals.
            let crack = ((a * 5.0 + b * 3.0 + hash(seed, x, y) * 6.0).fract() < 0.05)
                || (((a - b) * 4.0 + hash(seed ^ 3, x, y) * 6.0).fract() < 0.04);
            let colr = if crack {
                col(0xf2, 0xfb, 0xff)
            } else if v < 0.3 {
                col(0xa6, 0xcb, 0xe4)
            } else if v < 0.75 {
                col(0xbc, 0xdc, 0xee)
            } else {
                col(0xd6, 0xeb, 0xf7)
            };
            if hash(seed ^ 0x99, px, py) < melt {
                return;
            }
            dst.put_blended(px, py, tinted(colr, c.tint), 0.92);
        });
    }
}

fn traces(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let front = if fx.u < 1.0 {
        ramp(fx.u, 0.55, 1.0) * 0.12
    } else {
        0.12 + ramp(fx.u, 1.0, 1.92) * 0.88
    };
    let power = if fx.u < 2.05 { 1.0 } else { 1.0 - ramp(fx.u, 2.05, 2.5) };
    if front <= 0.0 {
        return;
    }
    for &(x, y, thr) in c.tiles {
        if thr > front || c.map.morphed(x, y) {
            continue;
        }
        let near = (1.0 - ((front - thr) / 0.06)).clamp(0.0, 1.0);
        let seed = fx.seed;
        tile_pixels(c, dst, x, y, |dst, px, py, a, b| {
            // A cold wash on the whole street, a bright trace along the middle
            // and a pulse travelling down it.
            let wash = 0.22 * power;
            let on_x = b.abs() < 0.035;
            let on_y = a.abs() < 0.035;
            if on_x || on_y {
                let phase = (c.t * 2.2 - (x + y) as f32 * 0.55 - (if on_x { a } else { b }) * 0.8).sin() * 0.5 + 0.5;
                let k = (0.4 + 0.6 * phase + near * 0.8).min(1.0) * power;
                if k > 0.12 {
                    let colr = if k > 0.8 {
                        col(0xd8, 0xfc, 0xff)
                    } else if k > 0.5 {
                        col(0x38, 0xe0, 0xff)
                    } else {
                        col(0x1a, 0x8a, 0xb0)
                    };
                    dst.put_blended(px, py, colr, 1.0);
                    return;
                }
            }
            if hash(seed, px, py) < 0.5 {
                dst.put_blended(px, py, col(0x10, 0x24, 0x38), wash);
            }
        });
    }
}

/// The flood's water front and, after it, the shoreline of the lagoon it leaves
/// (B50). B99c: this used to fill whole tiles with white at the crest, which at
/// 1x read as tile-shaped white blobs and, along the shore, as a hard white
/// zigzag traced round every remaining island — no water to see, and nothing
/// that said "the town is under water". The crest is now a band along the edge
/// the water is arriving by, and the waterline it leaves lies along the edges
/// that face dry land, broken and dimmed so it reads as surf and wet sand
/// rather than as an outline.
/// The flood's advancing crest (B50). B99c round 2: the *water itself* is the
/// lagoon class's own look now (`WaterKind::Lagoon` — pale, still, and lit at
/// night), so this pass draws only what the wave is doing: a bright crest along
/// the edge it is arriving by, the churn just behind it, and spray. Nothing is
/// drawn along the shore afterwards, and nothing is drawn over open water — a
/// per-tile foam line along every step of the tile grid read as a white sawtooth
/// traced round the lagoon, which is what the old pass did.
/// The flood's advancing crest (B50). B99c round 2: the *water itself* is the
/// lagoon class's own look now (`WaterKind::Lagoon` — pale, still and lit at
/// night), so this pass draws only what the wave is doing: a bright crest inside
/// the edge it is arriving by, the churn just behind it, and spray.
///
/// The crest sits a little way **inside** the tile and at a different offset on
/// each tile, never on the tile's own boundary. Drawn on the boundary it traced
/// the tile grid exactly, and the lagoon's shore came out as a white sawtooth
/// around every step of the coastline.
/// The flood's advancing bore (B50). B99c round 2: the *water itself* is the
/// lagoon class's own look now (`WaterKind::Lagoon` — pale, still and lit at
/// night), so this pass draws only what the wave is doing.
///
/// It is a wash over the whole leading ring of tiles, in the water's own pale
/// colour, fading back over the nine tiles the crest spans, plus spray — and
/// nothing along any tile boundary. Drawn as a line on the edge the water was
/// arriving by, it traced the tile grid exactly and the lagoon's shore came out
/// as a white sawtooth round every step of the old coastline; the shore is now
/// the shallow water's own pale rim, which follows the tiles without outlining
/// them.
fn foam(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    if fx.u < 0.98 || fx.u > 2.2 {
        return;
    }
    let frac = fx.frac;
    let seed = fx.seed;
    for &(x, y, thr) in c.tiles {
        if thr > frac || c.map.morphed(x, y) {
            continue;
        }
        let d = frac - thr;
        if d >= 0.09 {
            continue;
        }
        // How fresh the water is here: 1 at the very front, 0 nine tiles back.
        let fresh = 1.0 - (d / 0.09).clamp(0.0, 1.0);
        tile_pixels(c, dst, x, y, |dst, px, py, _a, _b| {
            // Dithered in 2-px blocks, like the spray above it: a bore, not a
            // tint over the whole tile.
            let h = hash(seed ^ 0xF04, (px + (c.t * 4.0) as i32) >> 1, py >> 1);
            if h < 0.30 + 0.45 * fresh {
                let colr = if h < 0.18 {
                    col(0xe6, 0xf4, 0xfb)
                } else if h < 0.55 {
                    col(0xb8, 0xd8, 0xe6)
                } else {
                    col(0x86, 0xa8, 0xb8)
                };
                dst.put_blended(px, py, tinted_floor(colr, c.tint, 0.60), 0.10 + 0.24 * fresh);
            }
        });
        // Spray above the crest, at the very front only.
        if fresh > 0.75 && fx.phase == phase::SPECTACLE {
            let (cx, cy) = tile_centre(c, x, y);
            for k in 0..3 {
                let ph = (c.t * 1.3 + hash(seed, x + k, y) * 3.0).fract();
                let sx = cx + (hash(seed, x, y + k) - 0.5) * 36.0;
                let sy = cy - ph * 18.0;
                dst.put_blended(sx as i32, sy as i32, col(0xff, 0xff, 0xff), 1.0 - ph);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Layer 3: smoke, fire, creatures, ships (over the world)
// ---------------------------------------------------------------------------

pub fn draw_world(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    // Fires and smoke over the lots the ending has hit.
    marks(dst, c);
    match fx.kind {
        kind::METEOR => meteor(dst, c),
        kind::VOLCANO => volcano(dst, c),
        kind::KAIJU => kaiju(dst, c),
        kind::ALIENS => aliens(dst, c),
        kind::AI => drones(dst, c),
        kind::MOON => moon_impact(dst, c),
        kind::SUNSET => lanterns(dst, c),
        _ => {}
    }
}

/// One lump of smoke or dust: a few overlapping blobs with a ragged edge, lit
/// from the upper left in three tones, so a column reads as billowing and not
/// as a stack of discs. `tone` is the grey level (0..1), `cov` the opacity.
fn puff(dst: &mut Framebuffer, c: &Ctx, cx: f32, cy: f32, r: f32, tone: f32, cov: f32, seed: u64, warm: f32) {
    if cov <= 0.02 || r < 0.8 {
        return;
    }
    let bucket = (((r / 2.0).round() as i32) * 2).clamp(2, 64);
    let variant = (hash(seed, 7, 7) * 4.0) as u8 & 3;
    let sp = puff_sprite(bucket, variant);
    let half = sp.0;
    let side = 2 * half + 1;
    let clip = dst.clip();
    let (ox, oy) = (cx as i32 - half, cy as i32 - half);
    let (x0, x1) = (ox.max(clip.x0), (ox + side).min(clip.x1));
    let (y0, y1) = (oy.max(clip.y0), (oy + side).min(clip.y1));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // Three tones and the edge tone, tinted once per puff, not per pixel.
    let mk = |t: f32| {
        let (r0, g0, b0) = ((t * 238.0 + warm * 30.0).min(255.0), (t * 228.0 + warm * 8.0).min(255.0), (t * 216.0 - warm * 14.0).max(0.0));
        tinted(col(r0 as u8, g0 as u8, b0 as u8), c.tint)
    };
    let tones = [0, mk(tone * 0.72), mk(tone), mk(tone * 1.18)];
    let solid = cov >= 0.97;
    for y in y0..y1 {
        let row = ((y - oy) * side) as usize;
        for x in x0..x1 {
            let code = sp.1[row + (x - ox) as usize];
            if code == 0 {
                continue;
            }
            let edge = code & 4 != 0;
            if edge && cov < bayer(x, y) * 1.2 {
                continue;
            }
            let colr = tones[(code & 3) as usize];
            dst.put_blended(x, y, colr, if edge || solid { 1.0 } else { cov });
        }
    }
}

thread_local! {
    static PUFFS: std::cell::RefCell<std::collections::HashMap<(i32, u8), std::rc::Rc<(i32, Vec<u8>)>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A pre-baked lump of smoke: a few overlapping blobs with a ragged edge, lit
/// from the upper left, as tone codes (1 dark, 2 mid, 3 light, +4 edge).
fn puff_sprite(bucket: i32, variant: u8) -> std::rc::Rc<(i32, Vec<u8>)> {
    PUFFS.with(|m| {
        if let Some(s) = m.borrow().get(&(bucket, variant)) {
            return s.clone();
        }
        let r = bucket as f32;
        let seed = 0x5EED_0000 + variant as u64 * 977;
        let half = (r * 1.45).ceil() as i32 + 2;
        let side = 2 * half + 1;
        let mut px = vec![0u8; (side * side) as usize];
        let nb = if r > 26.0 { 2 } else if r > 6.0 { 3 } else { 2 };
        for k in 0..nb {
            let ox = (hash(seed, k, 1) - 0.5) * r * 0.9;
            let oy = (hash(seed, k, 2) - 0.5) * r * 0.55;
            let rr = r * (0.55 + 0.4 * hash(seed, k, 3));
            for y in 0..side {
                for x in 0..side {
                    let (fx, fy) = ((x - half) as f32 + 0.5, (y - half) as f32 + 0.5);
                    let dx = (fx - ox) / rr;
                    let dy = (fy - oy) / (rr * 0.82);
                    let wob = 1.0 + (hash(seed ^ 0x55, x >> 1, y >> 1) - 0.5) * 0.4;
                    let d2 = (dx * dx + dy * dy) / (wob * wob);
                    if d2 >= 1.0 {
                        continue;
                    }
                    let lit = -(dx * 0.55 + dy * 0.85) + (hash(seed ^ 0x66, x, y) - 0.5) * 0.5;
                    let t = if lit > 0.35 { 3 } else if lit > -0.25 { 2 } else { 1 };
                    px[(y * side + x) as usize] = t | if d2 > 0.62 { 4 } else { 0 };
                }
            }
        }
        let rc = std::rc::Rc::new((half, px));
        m.borrow_mut().insert((bucket, variant), rc.clone());
        rc
    })
}

/// A column of smoke: round puffs rising and spreading, as pixel clusters.
fn smoke(dst: &mut Framebuffer, c: &Ctx, base: (f32, f32), height: f32, width: f32, seed: u64, dens: f32, dark: f32) {
    let n = 7;
    for i in 0..n {
        let ph = (c.t * 0.11 + i as f32 / n as f32 + hash(seed, 3, i) * 0.2).fract();
        let rise = ph * height;
        let drift = ph * ph * width * 1.4 + (c.t * 0.7 + i as f32).sin() * 1.5;
        let r = 2.5 + ph * width;
        let a = (1.0 - ph).powf(0.9) * dens;
        let tone = 0.2 + 0.36 * ph + 0.2 * (1.0 - dark);
        puff(dst, c, base.0 + drift, base.1 - rise, r, tone, a, seed ^ (i as u64) << 4, (1.0 - ph * 2.0).max(0.0));
    }
}

/// Only so many columns of smoke move every frame (the rest are lit but
/// still): the cost of a spectacle must not grow with the number of fires.
fn smoke_stride(c: &Ctx) -> usize {
    (c.marks.len() + 7) / 8
}

/// A bank of snow lying on the ground at `(x, y)`, `r` its size scale. Drawn
/// only where the static layer says there is *ground*: a drift stops at a wall
/// and never floats across a roof (B99c). Two shades and a soft bottom edge, so
/// a bank reads as settled snow rather than a white blob.
fn drift(dst: &mut Framebuffer, c: &Ctx, x: f32, y: f32, r: f32, seed: u64) {
    let (gx, gy) = ground_screen(c, x, y);
    // Snow lies along the wind, so a bank is long on one world axis: on screen
    // that is a shallow ellipse leaning the way that axis does.
    let lean = if hash(seed, 3, 3) < 0.5 { 1.0 } else { -1.0 };
    let rw = 8.0 * r;
    let rh = 3.2 * r;
    let clip = dst.clip();
    let y0 = (gy - rh - 1.0).ceil() as i32;
    let y1 = (gy + rh + 1.0).floor() as i32;
    for py in y0.max(clip.y0)..=y1.min(clip.y1 - 1) {
        let v = (py as f32 + 0.5 - gy) / rh;
        // The bank leans with the iso axis it lies along.
        let cx = gx + lean * v * 1.8;
        let x0 = (cx - rw * (1.0 - v * v).max(0.0).sqrt()).ceil() as i32;
        let x1 = (cx + rw * (1.0 - v * v).max(0.0).sqrt()).floor() as i32;
        for px in x0.max(clip.x0)..=x1.min(clip.x1 - 1) {
            if !ground_ok(c, px, py) {
                continue;
            }
            let u = (px as f32 + 0.5 - cx) / rw;
            // A dither at the edge, so the bank has no ruled outline.
            let edge = 1.0 - (u * u + v * v).min(1.0);
            if hash(seed ^ (px as u64) << 3, py, 9) > (edge * 6.0).min(1.0) {
                continue;
            }
            let lift = (-v * 0.5 + 0.5).clamp(0.0, 1.0);
            let tone = palette::mix(col(0xb7, 0xcb, 0xe2), col(0xf7, 0xfb, 0xff), lift);
            dst.put_blended(px, py, tinted(tone, c.tint), 1.0);
        }
    }
}

fn marks(dst: &mut Framebuffer, c: &Ctx) {
    let clip = dst.clip();
    let stride = smoke_stride(c).max(1);
    for (i, &(x, y, code, z)) in c.marks.iter().enumerate() {
        let (sx, sy) = c.cam.project(x, y, z as i32);
        if (sx as i32) < clip.x0 - 80 || (sx as i32) > clip.x1 + 80 || (sy as i32) < clip.y0 - 260 || (sy as i32) > clip.y1 + 40 {
            continue;
        }
        let seed = c.fx.seed ^ (i as u64) << 8;
        match code {
            1 => {
                if i % stride == 0 {
                    smoke(dst, c, (sx, sy), 55.0 + 25.0 * hash(seed, 1, 1), 8.0, seed, 0.9, 1.0);
                }
                let fl = 0.85;
                glow(dst, sx, sy + 4.0, 26.0, 16.0, (255.0, 120.0, 30.0), 0.55 * fl * (0.35 + 0.65 * c.glow));
            }
            7 => {
                // Snow banked against the building (B99c). The drifts used to be
                // four plain ellipses at the lot's centre, drawn over whatever
                // was there — so a white oval floated on a roof or in the air
                // and the whole city read as speckled with noise. Now they are
                // flattened banks around the *footprint*, and every pixel is
                // tested against the ground the static layer can see, so a drift
                // stops at the wall.
                let (beside, along) = (-1.0, 1.0);
                for k in 0..5 {
                    let side = if k % 2 == 0 { beside } else { along };
                    let off = 0.55 + 0.75 * hash(seed, k, 11);
                    let (ox, oy) = if k % 2 == 0 {
                        (side * off, (hash(seed, k, 12) - 0.5) * 1.2)
                    } else {
                        ((hash(seed, k, 12) - 0.5) * 1.2, side * off)
                    };
                    drift(dst, c, x + ox, y + oy, 0.7 + 0.8 * hash(seed, k, 13), seed ^ (k as u64) << 5);
                }
            }
            20 => {
                // A smokestack and its thick dark column.
                let base = c.cam.project(x + 0.9, y - 0.4, 0);
                dst.fill_rect(base.0 as i32 - 1, base.1 as i32 - z as i32, 4, z as i32, tinted(col(0x6a, 0x4a, 0x42), c.tint));
                dst.fill_rect(base.0 as i32 - 1, base.1 as i32 - z as i32, 4, 2, tinted(col(0xd8, 0xd0, 0xc0), c.tint));
                let top = (base.0 + 1.0, base.1 - z);
                let k = ramp(c.fx.u, 0.15, 1.0);
                if i % stride == 0 {
                    smoke(dst, c, top, 90.0 + 70.0 * k, 16.0 + 10.0 * k, seed, 0.95 * k, 1.0);
                }
            }
            2 => {
                // Smouldering rubble: a thin grey thread while it is fresh.
                if c.fx.u < 3.0 && hash(seed, 2, 2) < 0.6 && i % stride == 0 {
                    let fade = 1.0 - ramp(c.fx.u, 2.0, 3.0);
                    smoke(dst, c, (sx, sy), 38.0, 5.0, seed, 0.55 * fade, 0.5);
                }
            }
            _ => {}
        }
    }
}

// --- meteor ----------------------------------------------------------------

fn meteor(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let hit_p = 0.36;
    let site = ground_screen(c, fx.site.0, fx.site.1);
    let u = fx.u;
    if (1.0..1.0 + hit_p).contains(&u) {
        // The fireball: from far up-left to the ground, steepening as it comes.
        let t = (u - 1.0) / hit_p;
        let start = (site.0 - c.w as f32 * 0.48, -80.0f32);
        let e = t * t * (3.0 - 2.0 * t * 0.0);
        let head = (start.0 + (site.0 - start.0) * e, start.1 + (site.1 - start.1) * e);
        let size = 3.0 + 14.0 * e;
        // The trail: older positions, smaller, redder, then smoke.
        for k in (0..70).rev() {
            let tk = (t - k as f32 * 0.0035).max(0.0);
            let ek = tk * tk;
            let p = (start.0 + (site.0 - start.0) * ek, start.1 + (site.1 - start.1) * ek);
            let f = k as f32 / 70.0;
            let r = size * (1.0 - f * 0.75) * 0.9;
            let jitter = (hash(fx.seed, k, (c.t * 20.0) as i32) - 0.5) * 3.0 * f * size * 0.2;
            let colr = if f < 0.12 {
                col(0xff, 0xf4, 0xc4)
            } else if f < 0.4 {
                col(0xff, 0xb0, 0x38)
            } else if f < 0.7 {
                col(0xe4, 0x58, 0x1c)
            } else {
                tinted(col(0x6a, 0x5e, 0x58), c.tint)
            };
            ellipse(dst, p.0 + jitter, p.1, r, r, colr, 1.0 - f * 0.5, f > 0.4);
        }
        glow(dst, head.0, head.1, size * 6.0, size * 6.0, (255.0, 190.0, 100.0), 0.95);
        ellipse(dst, head.0, head.1, size, size, col(0xff, 0xe6, 0x9a), 1.0, false);
        ellipse(dst, head.0 + 1.0, head.1 + 1.0, size * 0.62, size * 0.62, col(0xff, 0xff, 0xf2), 1.0, false);
    }
    if u >= 1.0 + hit_p && u < 3.0 {
        let secs_since = (u - 1.0 - hit_p).max(0.0) * fx.dur.max(1.0);
        // The ring races out over the ground.
        let r_tiles = (secs_since * 0.05).min(26.0);
        let fade = 1.0 - ramp(r_tiles, 14.0, 26.0);
        if fade > 0.0 {
            ring(dst, c, site, r_tiles, fade);
        }
        // Dust and fire climb out of the crater.
        let rise = ramp(secs_since, 0.0, 200.0);
        let decay = 1.0 - ramp(u, 2.0, 3.0) * 0.8;
        for i in 0..16 {
            let ph = (i as f32 / 16.0 + hash(fx.seed, i, 5) * 0.1).fract();
            let hgt = rise * (30.0 + 150.0 * ph);
            let wob = (c.t * 0.5 + i as f32 * 1.3).sin() * (4.0 + ph * 12.0);
            let rr = 6.0 + ph * 20.0 + rise * 6.0;
            puff(dst, c, site.0 + wob, site.1 - hgt, rr, 0.22 + 0.3 * ph, 0.85 * decay, fx.seed ^ (i as u64) << 3, (0.5 - ph).max(0.0));
        }
        glow(dst, site.0, site.1 - 8.0, 80.0, 44.0, (255.0, 120.0, 40.0), 0.9 * decay * (0.5 + 0.5 * c.glow));
        // Flying debris: ballistic.
        if secs_since < 160.0 {
            for i in 0..60 {
                let a = hash(fx.seed, i, 8) * std::f32::consts::TAU;
                let sp = 40.0 + 90.0 * hash(fx.seed, i, 9);
                let vy = 80.0 + 160.0 * hash(fx.seed, i, 10);
                let tt = secs_since * 0.5;
                let x = site.0 + a.cos() * sp * tt * 0.5;
                let y = site.1 - (vy * tt - 70.0 * tt * tt);
                if y < site.1 + 4.0 {
                    let colr = if i % 3 == 0 { col(0xff, 0xa0, 0x30) } else { col(0x4a, 0x3a, 0x30) };
                    dst.put_blended(x as i32, y as i32, colr, 1.0);
                    dst.put_blended(x as i32 + 1, y as i32, colr, 1.0);
                }
            }
        }
        // The white of the impact.
        let flash = (1.0 - ramp(secs_since, 0.0, 22.0)).powf(1.6);
        if flash > 0.01 {
            let clip = dst.clip();
            dst.fill_blended(clip.x0, clip.y0, clip.width(), clip.height(), col(0xff, 0xf8, 0xe8), flash * 0.92);
        }
    }
}

/// An expanding ring on the ground, an ellipse in the iso view.
fn ring(dst: &mut Framebuffer, c: &Ctx, centre: (f32, f32), r_tiles: f32, fade: f32) {
    let rx = r_tiles * map::iso_consts::HALF_W as f32 * std::f32::consts::SQRT_2;
    let ry = r_tiles * map::iso_consts::HALF_H as f32 * std::f32::consts::SQRT_2;
    let steps = (rx * 2.5) as i32 + 12;
    for k in 0..steps {
        let th = k as f32 / steps as f32 * std::f32::consts::TAU;
        let (x, y) = (centre.0 + th.cos() * rx, centre.1 + th.sin() * ry);
        for w in 0..3 {
            let (xx, yy) = (x + th.cos() * w as f32, y + th.sin() * w as f32 * 0.5);
            let colr = match w {
                0 => col(0xff, 0xf6, 0xe0),
                1 => col(0xff, 0xc0, 0x60),
                _ => col(0x9a, 0x7a, 0x5a),
            };
            dst.put_blended(xx as i32, yy as i32, tinted(colr, if w == 0 { [1.0; 3] } else { c.tint }), fade * (1.0 - w as f32 * 0.25));
        }
        // Dust thrown up behind the front.
        if k % 3 == 0 {
            let up = 3.0 + 6.0 * hash(c.fx.seed, k, 3);
            dst.put_blended(x as i32, (y - up) as i32, tinted(col(0x8a, 0x7a, 0x68), c.tint), fade * 0.6);
        }
    }
}

// --- volcano -----------------------------------------------------------------

fn volcano(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let vent = vent_screen(c);
    let u = fx.u;
    let build = ramp(u, 0.1, 0.95);
    // Smoke thread and the red mouth from the warning on.
    if u < 1.0 {
        let k = ramp(u, 0.2, 1.0);
        smoke(dst, c, (vent.0, vent.1 - 6.0), 30.0 + 90.0 * k, 4.0 + 10.0 * k, fx.seed, 0.55 + 0.4 * k, 1.0);
        glow(dst, vent.0, vent.1 - 2.0, 14.0 + 30.0 * k, 8.0 + 16.0 * k, (255.0, 80.0, 20.0), 0.9 * k * (0.35 + 0.65 * c.glow));
        let _ = build;
        return;
    }
    // The eruption.
    let erupt = ramp(u, 1.0, 1.12) * (1.0 - 0.7 * ramp(u, 1.9, 3.0));
    let quiet = 1.0 - ramp(u, 2.0, 3.0);
    if erupt <= 0.0 {
        return;
    }
    // The ash column: a stack of dark puffs that widen as they climb and
    // spread into a flat top.
    let height = 180.0 + 200.0 * erupt;
    let n = 28;
    for i in 0..n {
        let ph = (c.t * 0.07 + i as f32 / n as f32).fract();
        let hgt = ph * height;
        let spread = 6.0 + ph * ph * 62.0 + 6.0 * (ph * 8.0).sin().abs();
        let side = (hash(fx.seed, i, 21) - 0.5) * spread * 1.1 + ph * ph * 40.0;
        let r = 10.0 + ph * 40.0;
        puff(dst, c, vent.0 + side, vent.1 - 10.0 - hgt, r, 0.10 + 0.24 * ph, (0.95 - ph * 0.2) * erupt, fx.seed ^ (i as u64) << 5, (0.45 - ph).max(0.0) * 2.0);
    }
    // Lightning in the column.
    let bolt = (c.t * 0.9).fract();
    if bolt < 0.1 && erupt > 0.5 {
        let bx = vent.0 + (hash(fx.seed, (c.t * 0.9) as i32, 4) - 0.5) * 60.0;
        let mut y = vent.1 - 40.0;
        let mut x = bx;
        for _ in 0..16 {
            dst.put_blended(x as i32, y as i32, col(0xf4, 0xf0, 0xff), 1.0);
            dst.put_blended(x as i32 + 1, y as i32, col(0xc0, 0xb8, 0xff), 0.8);
            y -= 5.0;
            x += (hash(fx.seed, (y as i32) + (c.t * 3.0) as i32, 5) - 0.5) * 8.0;
        }
    }
    // The fountain: lava thrown up and falling back.
    for i in 0..46 {
        let a = hash(fx.seed, i, 31) * std::f32::consts::TAU;
        let sp = 8.0 + 38.0 * hash(fx.seed, i, 32);
        let vy = 110.0 + 120.0 * hash(fx.seed, i, 33);
        let per = 2.4 + 1.4 * hash(fx.seed, i, 34);
        let tt = ((c.t + hash(fx.seed, i, 35) * per) % per) * 1.0;
        let x = vent.0 + a.cos() * sp * tt * 0.6;
        let y = vent.1 - 8.0 - (vy * tt * 0.55 - 52.0 * tt * tt);
        if y < vent.1 + 6.0 {
            let f = tt / per;
            let colr = if f < 0.4 { col(0xff, 0xe0, 0x70) } else if f < 0.75 { col(0xff, 0x90, 0x28) } else { col(0xc8, 0x3a, 0x14) };
            dst.put_blended(x as i32, y as i32, colr, erupt);
            dst.put_blended(x as i32, y as i32 + 1, colr, erupt * 0.8);
        }
    }
    glow(dst, vent.0, vent.1 - 6.0, 70.0, 40.0, (255.0, 100.0, 24.0), 0.95 * erupt * (0.5 + 0.5 * c.glow));
    // Ash falling over the whole picture.
    ash_fall(dst, c, 0.9 * erupt * quiet.max(0.25), vent);
}

fn ash_fall(dst: &mut Framebuffer, c: &Ctx, k: f32, vent: (f32, f32)) {
    let clip = dst.clip().intersect(&ash_rect(c, vent));
    let n = (360.0 * k) as i32;
    for i in 0..n {
        let r = ash_rect(c, vent);
        let x0 = r.x0 as f32 + hash(c.fx.seed, i, 41) * r.width() as f32;
        let sp = 14.0 + 20.0 * hash(c.fx.seed, i, 42);
        let y = r.y0 as f32 + ((hash(c.fx.seed, i, 43) * r.height() as f32) + c.t * sp) % r.height() as f32;
        let x = x0 + (c.t * 0.6 + i as f32).sin() * 3.0 - y * 0.08;
        let (xi, yi) = (x as i32, y as i32);
        let lower = xi >= vent.0 as i32 - 70 && xi < vent.0 as i32 + 120 && yi >= vent.1 as i32 - 210;
        if lower && xi >= clip.x0 && xi < clip.x1 && yi >= clip.y0 && yi < clip.y1 {
            let v = 0.5 + 0.4 * hash(c.fx.seed, i, 44);
            dst.put_blended(x as i32, y as i32, col((v * 150.0) as u8, (v * 142.0) as u8, (v * 138.0) as u8), 0.8);
        }
    }
}

// --- kaiju ---------------------------------------------------------------------

/// The Hollowback: a dome-shelled strider on four jointed legs, a long neck
/// and a small lantern-eyed head, crystal spines along its back. Drawn side-on
/// and mirrored for the way it walks.
fn hollowback(dst: &mut Framebuffer, c: &Ctx, foot: (f32, f32), g: &endgeo::Giant, variant: u8, scale: f32, alpha: f32) {
    let flip = {
        let sx = g.head.0 - g.head.1;
        if sx >= 0.0 { 1.0f32 } else { -1.0 }
    };
    // Toward or away from the viewer shifts the head and the shell a little.
    let sy = g.head.0 + g.head.1;
    let depth_lean = sy.clamp(-1.0, 1.0);
    let t = c.t;
    let fight = g.mode == endgeo::GiantMode::Fight;
    let fall = if g.mode == endgeo::GiantMode::Fall { g.fall } else { 0.0 };
    let walk_phase = g.walked * 0.55;
    let (shell_d, shell_m, shell_l, shell_h) = if variant == 0 {
        (col(0x2c, 0x3d, 0x38), col(0x46, 0x61, 0x57), col(0x6a, 0x8a, 0x78), col(0x9a, 0xb8, 0x9c))
    } else {
        (col(0x44, 0x2c, 0x34), col(0x6a, 0x42, 0x4c), col(0x92, 0x62, 0x68), col(0xbc, 0x8c, 0x84))
    };
    let skin_d = col(0x48, 0x44, 0x3c);
    let skin_l = col(0x74, 0x6e, 0x60);
    let bone = col(0xc9, 0xc0, 0xa6);
    let outline = col(0x0e, 0x12, 0x12);
    let to = |lx: f32, ly: f32| -> (f32, f32) { (foot.0 + flip * lx * scale, foot.1 - ly * scale) };
    let tn = |col: Color| tinted(col, c.tint);
    let sink = fall * 30.0;
    let bob = if fight { (t * 3.0).sin() * 2.0 } else { (walk_phase * 2.0).sin().abs() * 2.0 };
    let body_y = 50.0 - sink + bob;

    // Legs: far pair first (darker), near pair last.
    let legs = [(-17.0f32, 0.0f32, true), (15.0, 3.14, true), (-19.0, 3.14, false), (17.0, 0.0, false)];
    for (hx, off, far) in legs {
        let ph = walk_phase + off;
        let swing = if fight { 4.0 * (t * 2.0 + off).sin() } else { ph.sin() * 11.0 };
        let lift = if fight { 0.0 } else { (ph.cos()).max(0.0) * 7.0 };
        let bend = 1.0 - fall * 0.6;
        let hip = (hx, body_y - 4.0);
        let knee = (hx + 7.0 + swing * 0.4, (26.0 - sink * 0.2) * bend + lift * 0.6);
        let ft = (hx + swing, (lift * 0.9).max(0.0) * 0.0 + lift);
        let (c0, c1) = if far { (skin_d, col(0x32, 0x2e, 0x28)) } else { (skin_l, skin_d) };
        let upper = limb(to(hip.0, hip.1), to(knee.0, knee.1), 6.0 * scale, 4.5 * scale);
        poly(dst, &upper, tn(c0), tn(outline));
        let lower = limb(to(knee.0, knee.1), to(ft.0, ft.1), 4.0 * scale, 3.0 * scale);
        poly(dst, &lower, tn(c1), tn(outline));
        // The knuckle and the foot pad.
        let k = to(knee.0, knee.1);
        dst.fill_rect(k.0 as i32 - 1, k.1 as i32 - 1, 3, 3, tn(bone));
        let f = to(ft.0, ft.1);
        dst.fill_rect(f.0 as i32 - 3, f.1 as i32 - 1, 7, 3, tn(col(0x20, 0x1e, 0x1a)));
        // Dust when a foot lands.
        if !fight && ph.cos() < -0.92 && g.mode == endgeo::GiantMode::Walk {
            let k2 = (ph.cos() + 1.0) / 0.08;
            let r = 5.0 + (1.0 - k2) * 14.0;
            ellipse(dst, f.0, f.1, r * 1.6, r * 0.55, tn(col(0xa8, 0x98, 0x80)), 0.45 * k2.clamp(0.0, 1.0), true);
        }
    }
    // Tail, swinging, ending in a glowing bulb.
    let sway = (t * 1.1).sin() * 4.0 + if fight { (t * 3.0).sin() * 8.0 } else { 0.0 };
    let t0 = to(-30.0, body_y + 2.0);
    let t1 = to(-52.0, body_y - 12.0 + sway * 0.3);
    let t2 = to(-70.0 - fall * 6.0, body_y - 22.0 + sway - sink * 0.6);
    poly(dst, &limb(t0, t1, 9.0 * scale, 6.0 * scale), tn(skin_d), tn(outline));
    poly(dst, &limb(t1, t2, 6.0 * scale, 3.0 * scale), tn(skin_d), tn(outline));
    ellipse(dst, t2.0, t2.1, 4.0 * scale, 4.0 * scale, tn(shell_m), 1.0, false);
    ellipse(dst, t2.0, t2.1, 2.2 * scale, 2.2 * scale, col(0x9e, 0xf6, 0xff), 1.0, false);
    glow(dst, t2.0, t2.1, 14.0, 12.0, (110.0, 230.0, 255.0), 0.55);
    // The shell: a dome, three tones and a ridge.
    let mut dome: Vec<(f32, f32)> = Vec::new();
    for i in 0..=18 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        dome.push(to(a.cos() * 36.0, body_y + 6.0 + a.sin() * 26.0));
    }
    poly(dst, &dome, tn(shell_m), tn(outline));
    // Lower, shadowed band.
    let mut under: Vec<(f32, f32)> = Vec::new();
    for i in 0..=18 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        under.push(to(a.cos() * 36.0, body_y + 6.0 + a.sin() * 26.0 * 0.0 + 2.0 - a.sin() * 0.0));
    }
    let belly = [to(-36.0, body_y + 8.0), to(36.0, body_y + 8.0), to(30.0, body_y - 6.0), to(-30.0, body_y - 6.0)];
    poly(dst, &belly, tn(skin_d), tn(outline));
    // Re-draw the dome's face over the belly seam.
    let mut face: Vec<(f32, f32)> = Vec::new();
    for i in 0..=18 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        face.push(to(a.cos() * 35.0, body_y + 8.0 + a.sin() * 25.0));
    }
    dst.fill_poly(&face, tn(shell_m));
    // Highlight arc, shadow arc, scutes.
    let mut hi: Vec<(f32, f32)> = Vec::new();
    for i in 2..=11 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        hi.push(to(a.cos() * 30.0 + 4.0, body_y + 9.0 + a.sin() * 22.0));
    }
    for i in (2..=11).rev() {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        hi.push(to(a.cos() * 25.0 + 4.0, body_y + 9.0 + a.sin() * 17.0));
    }
    dst.fill_poly(&hi, tn(shell_l));
    for i in 3..9 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        let p = to(a.cos() * 27.0 + 3.0, body_y + 10.0 + a.sin() * 20.0);
        dst.put(p.0 as i32, p.1 as i32, tn(shell_h));
    }
    let mut sh: Vec<(f32, f32)> = Vec::new();
    for i in 11..=18 {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        sh.push(to(a.cos() * 35.0, body_y + 8.0 + a.sin() * 25.0));
    }
    for i in (11..=18).rev() {
        let a = std::f32::consts::PI * i as f32 / 18.0;
        sh.push(to(a.cos() * 27.0, body_y + 8.0 + a.sin() * 18.0));
    }
    dst.fill_poly(&sh, tn(shell_d));
    // Barnacle scutes: a lattice of lighter dots.
    for gy in 0..5 {
        for gx in 0..9 {
            let (lx, ly) = (-30.0 + gx as f32 * 7.5 + (gy % 2) as f32 * 3.5, body_y + 6.0 + gy as f32 * 5.0);
            let inside = ((lx / 35.0).powi(2) + ((ly - body_y - 8.0) / 25.0).powi(2)) < 0.8;
            if inside && ly > body_y + 6.0 {
                let p = to(lx, ly);
                dst.put(p.0 as i32, p.1 as i32, tn(shell_d));
            }
        }
    }
    // Crystal spines along the back, glowing.
    let flare = if fight { 1.5 } else { 1.0 };
    for i in 0..7 {
        let a = std::f32::consts::PI * (0.12 + i as f32 * 0.12);
        let base = (a.cos() * 30.0 + 4.0, body_y + 9.0 + a.sin() * 24.0);
        let len = (9.0 + 7.0 * hash(c.fx.seed, i, 7)) * flare * (1.0 - fall * 0.7);
        let lean = -0.35 - (i as f32 - 3.0) * 0.1;
        let tip = (base.0 + lean * len, base.1 + len);
        let b0 = to(base.0 - 2.0, base.1 - 1.0);
        let b1 = to(base.0 + 2.0, base.1 - 1.0);
        let pulse = 0.7 + 0.3 * (t * 2.0 + i as f32).sin();
        dst.fill_poly(&[b0, b1, to(tip.0, tip.1)], col(0x9e, 0xf6, 0xff));
        let tp = to(tip.0, tip.1);
        dst.put(tp.0 as i32, tp.1 as i32, col(0xff, 0xff, 0xff));
        let mid = to(base.0 + lean * len * 0.4, base.1 + len * 0.4);
        dst.put(mid.0 as i32, mid.1 as i32, col(0x4f, 0xd3, 0xe8));
        glow(dst, tp.0, tp.1, 9.0, 9.0, (90.0, 220.0, 255.0), 0.55 * pulse);
    }
    // The neck and the head.
    let weave = if fight { (t * 2.6).sin() * 10.0 } else { (t * 0.8).sin() * 2.5 };
    let sh0 = to(30.0, body_y + 14.0);
    let n1 = to(42.0 + weave * 0.3, body_y + 30.0 - fall * 8.0);
    let hd = (54.0 + weave, body_y + 44.0 - fall * 34.0 - depth_lean * 1.5);
    let h0 = to(hd.0, hd.1);
    poly(dst, &limb(sh0, n1, 9.0 * scale, 6.0 * scale), tn(skin_l), tn(outline));
    poly(dst, &limb(n1, h0, 6.0 * scale, 5.0 * scale), tn(skin_l), tn(outline));
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let p = (sh0.0 + (h0.0 - sh0.0) * f, sh0.1 + (h0.1 - sh0.1) * f);
        dst.put(p.0 as i32, p.1 as i32 - 1, tn(skin_d));
    }
    // The head: a wedge with a snout, a bone brow, the amber eye.
    let snout = if fight && (t * 3.0).sin() > 0.4 { 3.0 } else { 0.0 };
    let head = [
        to(hd.0 - 4.0, hd.1 + 6.0),
        to(hd.0 + 10.0, hd.1 + 3.0),
        to(hd.0 + 19.0, hd.1 - 1.0 - snout * 0.5),
        to(hd.0 + 18.0 + snout, hd.1 - 5.0 - snout),
        to(hd.0 + 8.0, hd.1 - 6.0),
        to(hd.0 - 4.0, hd.1 - 4.0),
    ];
    poly(dst, &head, tn(skin_l), tn(outline));
    // The jaw, a darker wedge under it.
    let jaw = [to(hd.0 + 2.0, hd.1 - 4.0), to(hd.0 + 17.0, hd.1 - 4.0 - snout), to(hd.0 + 9.0, hd.1 - 9.0 - snout * 0.5)];
    dst.fill_poly(&jaw, tn(skin_d));
    let brow = to(hd.0 + 1.0, hd.1 + 4.0);
    dst.fill_rect(brow.0 as i32 - 1, brow.1 as i32 - 1, 7, 2, tn(bone));
    let eye = to(hd.0 + 5.0, hd.1 + 1.0);
    dst.fill_rect(eye.0 as i32, eye.1 as i32, 2, 2, col(0xff, 0xc2, 0x4a));
    dst.put(eye.0 as i32, eye.1 as i32, col(0xff, 0xf0, 0xb0));
    glow(dst, eye.0, eye.1, 12.0, 10.0, (255.0, 170.0, 60.0), 0.7);
    // The lantern organ under the chin.
    let lan = to(hd.0 + 6.0, hd.1 - 8.0);
    glow(dst, lan.0, lan.1, 10.0, 8.0, (255.0, 190.0, 90.0), 0.5 * (0.7 + 0.3 * (t * 4.0).sin()));
    dst.put(lan.0 as i32, lan.1 as i32 + 1, col(0xff, 0xd8, 0x80));
    let _ = (shell_h, alpha);
}

fn kaiju(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let u = fx.u;
    // The warning: a shape in the haze, standing on the road in from the edge.
    if u < 0.82 {
        if u > 0.45 {
            let k = ramp(u, 0.45, 0.8);
            let (a, b) = ((fx.aux[0], fx.aux[1]), (fx.aux[2], fx.aux[3]));
            let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1.0);
            let head = ((b.0 - a.0) / l, (b.1 - a.1) / l);
            let p = (a.0 + head.0 * 6.0, a.1 + head.1 * 6.0);
            let foot = ground_screen(c, p.0, p.1);
            if foot.1 > c.sky_h as f32 + 24.0 {
                let g = endgeo::Giant { x: p.0, y: p.1, dir: 0, head, walked: 0.0, mode: endgeo::GiantMode::Walk, fall: 0.0, which: 0 };
                let mut tint = c.tint;
                for t in tint.iter_mut() {
                    *t = *t * 0.5 + 0.38 * k; // lost in the haze, then clearer
                }
                let c2 = Ctx { tint, ..*c };
                hollowback(dst, &c2, foot, &g, 0, 0.6, 0.5);
            }
        }
        return;
    }
    let giants = endgeo::giants(&fx.aux, u);
    // Painter's order: the one farther up the screen first.
    let mut order: Vec<&endgeo::Giant> = giants.iter().collect();
    order.sort_by(|a, b| (a.x + a.y).partial_cmp(&(b.x + b.y)).unwrap_or(std::cmp::Ordering::Equal));
    for g in order {
        if g.mode == endgeo::GiantMode::Gone {
            continue;
        }
        let foot = ground_screen(c, g.x, g.y);
        let sink = if g.mode == endgeo::GiantMode::Fall { g.fall } else { 0.0 };
        hollowback(dst, c, (foot.0, foot.1 + sink * 6.0), g, g.which, 1.0, 1.0);
        if g.mode == endgeo::GiantMode::Fall {
            // It comes apart in a spray of light.
            for i in 0..40 {
                let a = hash(fx.seed, i, 61) * std::f32::consts::TAU;
                let r = g.fall * (30.0 + 80.0 * hash(fx.seed, i, 62));
                let (x, y) = (foot.0 + a.cos() * r, foot.1 - 40.0 + a.sin() * r * 0.6 - g.fall * 30.0);
                dst.put_blended(x as i32, y as i32, col(0x9e, 0xf6, 0xff), 1.0 - g.fall * 0.6);
            }
        }
        if g.mode == endgeo::GiantMode::Fight {
            // Sparks where they meet.
            let m = ground_screen(c, g.x + g.head.0 * 1.8, g.y + g.head.1 * 1.8);
            for i in 0..14 {
                let ph = (c.t * 2.0 + hash(fx.seed, i, 71)).fract();
                let a = hash(fx.seed, i, 72) * std::f32::consts::TAU;
                let r = ph * 24.0;
                dst.put_blended((m.0 + a.cos() * r) as i32, (m.1 - 48.0 + a.sin() * r) as i32, col(0xff, 0xe8, 0xa0), 1.0 - ph);
            }
            glow(dst, m.0, m.1 - 48.0, 26.0, 20.0, (255.0, 230.0, 160.0), 0.7 * (0.5 + 0.5 * (c.t * 11.0).sin().abs()));
        }
    }
    // Footprint dust trails are in the tiles; the smoke is in the marks.
}

// --- aliens ---------------------------------------------------------------------

fn saucer(dst: &mut Framebuffer, c: &Ctx, s: &endgeo::Saucer, ground: (f32, f32)) {
    let alt = s.alt * 0.62;
    let (cx, cy) = (ground.0, ground.1 - alt);
    let t = c.t;
    // Beam first, so the hull sits over its top.
    if s.beam > 0.03 {
        let top = (cy + 7.0, 7.0f32);
        let bot = (ground.1, 30.0f32);
        let h = (bot.0 - top.0).max(1.0);
        for y in top.0 as i32..bot.0 as i32 {
            let f = (y as f32 - top.0) / h;
            let half = top.1 + (bot.1 - top.1) * f;
            let x0 = (cx - half) as i32;
            let x1 = (cx + half) as i32;
            for x in x0..=x1 {
                let edge = ((x as f32 - cx).abs() / half).powi(2);
                let stripe = if (x + (t * 12.0) as i32 / 2) % 5 == 0 { 0.14 } else { 0.0 };
                let band = ((y as f32 * 0.35 - t * 9.0).sin() * 0.5 + 0.5) * 0.12;
                let a = (0.28 + stripe + band + edge * 0.2) * s.beam * (0.6 + 0.4 * f);
                if a > bayer(x, y) * 0.85 {
                    add(dst, x, y, (190.0, 250.0, 255.0), 0.55 * s.beam);
                }
            }
        }
        // The pool of light on the ground and things rising in it.
        ellipse(dst, ground.0, ground.1, 31.0, 12.0, col(0xd8, 0xfb, 0xff), 0.5 * s.beam, true);
        ellipse(dst, ground.0, ground.1, 21.0, 8.0, col(0xf2, 0xff, 0xff), 0.7 * s.beam, true);
        for i in 0..9 {
            let ph = (t * 0.22 + i as f32 * 0.113 + hash(c.fx.seed, i + s.id as i32 * 9, 3)).fract();
            let y = ground.1 - ph * (ground.1 - cy - 6.0);
            let x = ground.0 + (hash(c.fx.seed, i, s.id as i32) - 0.5) * 28.0 * (1.0 - ph * 0.7);
            let kind = (hash(c.fx.seed, i, 99 + s.id as i32) * 3.0) as i32;
            let colr = match kind {
                0 => col(0x7a, 0x4a, 0x3a),
                1 => col(0x3a, 0x6a, 0x3a),
                _ => col(0x5a, 0x5a, 0x70),
            };
            let sz = 1.0 + (1.0 - ph) * 1.5;
            dst.fill_rect(x as i32, y as i32, (sz * 2.0) as i32, (sz * 1.6) as i32, tinted(colr, c.tint));
            dst.put_blended(x as i32, y as i32 - 1, col(0xff, 0xff, 0xff), 0.8 * s.beam);
        }
    }
    // The shadow it throws on the ground.
    if s.alt < 400.0 {
        ellipse(dst, ground.0 + 6.0, ground.1 + 2.0, 34.0, 11.0, col(0x10, 0x14, 0x20), 0.28 * (1.0 - s.alt / 400.0), true);
    }
    let (hull_d, hull_m, hull_l) = (col(0x2e, 0x36, 0x48), col(0x58, 0x66, 0x7e), col(0x9a, 0xa8, 0xc0));
    let outline = col(0x0a, 0x0e, 0x18);
    let tn = |x: Color| tinted(x, c.tint);
    // The lower hull, then the disc, then the dome.
    let under: Vec<(f32, f32)> = (0..=12)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 12.0;
            (cx - a.cos() * 22.0, cy + 2.0 + a.sin() * 9.0)
        })
        .collect();
    poly(dst, &under, tn(hull_d), tn(outline));
    let disc: Vec<(f32, f32)> = (0..24)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 24.0;
            (cx + a.cos() * 32.0, cy + a.sin() * 8.0)
        })
        .collect();
    poly(dst, &disc, tn(hull_m), tn(outline));
    // The lit top surface and the rim shadow.
    let top: Vec<(f32, f32)> = (0..24)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 24.0;
            (cx + a.cos() * 27.0, cy - 1.0 + a.sin() * 5.0)
        })
        .collect();
    dst.fill_poly(&top, tn(hull_l));
    let dome: Vec<(f32, f32)> = (0..=10)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 10.0;
            (cx - a.cos() * 11.0, cy - 2.0 - a.sin() * 9.0)
        })
        .collect();
    poly(dst, &dome, col(0x7a, 0xd8, 0xe8), tn(outline));
    let glass: Vec<(f32, f32)> = (1..=7)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 10.0;
            (cx - a.cos() * 8.0 - 1.0, cy - 3.0 - a.sin() * 6.0)
        })
        .collect();
    for p in &glass {
        dst.put(p.0 as i32, p.1 as i32, col(0xd8, 0xfa, 0xff));
    }
    // Rim lights, chasing round.
    for i in 0..12 {
        let a = std::f32::consts::TAU * i as f32 / 12.0;
        let (x, y) = (cx + a.cos() * 30.0, cy + 1.0 + a.sin() * 6.0);
        let on = ((t * 3.0 + i as f32 * 0.52).sin() > 0.0) as i32;
        let colr = if (i + on) % 2 == 0 { col(0x6a, 0xf0, 0xff) } else { col(0xff, 0xc8, 0x5a) };
        if y > cy - 1.0 {
            dst.put(x as i32, y as i32, colr);
            dst.put(x as i32 + 1, y as i32, colr);
        }
    }
    // The engine glow under the hull.
    glow(dst, cx, cy + 9.0, 30.0, 14.0, (150.0, 240.0, 255.0), 0.5 + 0.4 * s.beam);
}

fn aliens(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let n = fx.aux[2].max(3.0) as usize;
    let ships = endgeo::saucers(fx.site, fx.aux[1], n, fx.seed, fx.u, c.t);
    let mut order: Vec<&endgeo::Saucer> = ships.iter().collect();
    order.sort_by(|a, b| (a.x + a.y).partial_cmp(&(b.x + b.y)).unwrap_or(std::cmp::Ordering::Equal));
    for s in order {
        let g = ground_screen(c, s.x, s.y);
        saucer(dst, c, s, g);
    }
}

// --- drones -----------------------------------------------------------------------

fn drones(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let power = if fx.u < 2.05 { 1.0 } else { 1.0 - ramp(fx.u, 2.05, 2.4) };
    if power <= 0.0 {
        return;
    }
    for (i, &(x, y)) in c.drones.iter().enumerate() {
        let (sx, sy) = c.cam.project(x, y, crate::iso::level_px(c.map.surface_level_at(x, y)) + 9);
        let bob = (c.t * 2.4 + i as f32 * 0.9).sin() * 1.5;
        let (px, py) = (sx as i32, (sy + bob) as i32);
        // A small quad-rotor: dark body, four blurred rotors, a cyan eye.
        dst.put(px, py, col(0x28, 0x34, 0x48));
        dst.put(px - 1, py, col(0x3c, 0x4c, 0x66));
        dst.put(px + 1, py, col(0x3c, 0x4c, 0x66));
        dst.put(px, py + 1, col(0x1a, 0x22, 0x30));
        let spin = ((c.t * 30.0 + i as f32) as i32) & 1;
        let rot = col(0x8a, 0x9c, 0xb4);
        dst.put(px - 3 + spin, py - 1, rot);
        dst.put(px - 2 + spin, py - 1, rot);
        dst.put(px + 2 - spin, py - 1, rot);
        dst.put(px + 3 - spin, py - 1, rot);
        dst.put(px, py, col(0x38, 0xe0, 0xff));
        add(dst, px, py, (50.0, 220.0, 255.0), 0.5 * power);
        // A shadow on the street.
        let g = c.cam.project(x, y, crate::iso::level_px(c.map.surface_level_at(x, y)));
        dst.put_blended(g.0 as i32, g.1 as i32, col(0x10, 0x14, 0x1c), 0.3);
        dst.put_blended(g.0 as i32 + 1, g.1 as i32, col(0x10, 0x14, 0x1c), 0.3);
    }
}

// --- falling moon -----------------------------------------------------------------

/// The embers falling out of the moon's impact (B61e): the pixel positions, so
/// the draw and the damage rectangles cannot disagree about where they are.
/// Each one is drawn as two pixels, one of them on the row above.
fn moon_ember_positions(c: &Ctx) -> Vec<(i32, i32)> {
    let fx = c.fx;
    let u = fx.u;
    if !(1.0..1.7).contains(&u) {
        return Vec::new();
    }
    let k = ramp(u, 1.0, 1.7);
    let n = (80.0 * k) as i32;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        let x = hash(fx.seed, i, 81) * c.w as f32;
        let y0 = hash(fx.seed, i, 82) * c.sky_h as f32 * 0.8;
        let ph = (c.t * 0.5 + hash(fx.seed, i, 83)).fract();
        let y = y0 + ph * (c.h as f32 - y0) * 0.9;
        out.push((x as i32, y as i32));
    }
    out
}

fn moon_impact(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    let u = fx.u;
    if u < 1.0 {
        return;
    }
    let site = ground_screen(c, fx.site.0, fx.site.1);
    // Warning light falls on the ground as it comes in; embers fall from it.
    if u < 1.7 {
        for (i, (x, y)) in moon_ember_positions(c).into_iter().enumerate() {
            let ph = (c.t * 0.5 + hash(fx.seed, i as i32, 83)).fract();
            dst.put_blended(x, y, col(0xff, 0xa0, 0x50), 1.0 - ph);
            dst.put_blended(x, y - 1, col(0xff, 0xe0, 0xa0), (1.0 - ph) * 0.7);
        }
        return;
    }
    // The impact: the biggest white of all, a ring of fire racing outward.
    let secs = (u - 1.7).max(0.0) * fx.dur.max(1.0);
    let r_tiles = (secs * 0.08).min(46.0);
    if r_tiles < 40.0 {
        ring(dst, c, site, r_tiles, 1.0 - ramp(r_tiles, 28.0, 46.0));
        ring(dst, c, site, (r_tiles * 0.8).max(0.0), (1.0 - ramp(r_tiles, 20.0, 46.0)) * 0.7);
    }
    let flash = (1.0 - ramp(secs, 0.0, 60.0)).powf(1.5);
    if flash > 0.01 {
        let clip = dst.clip();
        dst.fill_blended(clip.x0, clip.y0, clip.width(), clip.height(), col(0xff, 0xfa, 0xf0), flash * 0.95);
    }
    let decay = 1.0 - ramp(u, 2.0, 3.0) * 0.8;
    glow(dst, site.0, site.1 - 10.0, 140.0, 80.0, (255.0, 130.0, 50.0), 0.9 * decay * (0.5 + 0.5 * c.glow));
    // A pillar of dust and fire.
    for i in 0..18 {
        let ph = (i as f32 / 18.0 + hash(fx.seed, i, 91) * 0.1).fract();
        let hgt = ramp(secs, 0.0, 300.0) * (20.0 + 220.0 * ph);
        let rr = 9.0 + ph * 34.0;
        puff(dst, c, site.0 + (c.t * 0.4 + i as f32).sin() * 10.0, site.1 - hgt, rr, 0.22 + 0.3 * ph, 0.8 * decay, fx.seed ^ (i as u64) << 6, (0.5 - ph).max(0.0));
    }
}

// --- sunset ----------------------------------------------------------------------

fn lanterns(dst: &mut Framebuffer, c: &Ctx) {
    let fx = c.fx;
    if fx.u < 1.1 || fx.u > 2.6 {
        return;
    }
    let k = ramp(fx.u, 1.1, 1.5) * (1.0 - ramp(fx.u, 2.2, 2.6));
    let (sx, sy) = ground_screen(c, fx.site.0, fx.site.1);
    for i in 0..40 {
        let ph = (c.t * 0.025 + hash(fx.seed, i, 1)).fract();
        let x = sx + (hash(fx.seed, i, 2) - 0.5) * 420.0 + (c.t * 0.5 + i as f32).sin() * 6.0;
        let y = sy + 40.0 - ph * 520.0;
        if y < 4.0 {
            continue;
        }
        let a = k * (1.0 - ph * 0.5);
        dst.put_blended(x as i32, y as i32, col(0xff, 0xc8, 0x70), a);
        dst.put_blended(x as i32, y as i32 + 1, col(0xff, 0x90, 0x40), a);
        dst.put_blended(x as i32 - 1, y as i32, col(0xff, 0xa8, 0x50), a * 0.6);
        dst.put_blended(x as i32 + 1, y as i32, col(0xff, 0xa8, 0x50), a * 0.6);
        glow(dst, x, y, 7.0, 7.0, (255.0, 170.0, 70.0), 0.6 * a);
    }
}

/// The union of the screen rectangles worth redrawing for an ending: the whole
/// frame (every layer here can touch any pixel).
pub fn damage(w: usize, h: usize) -> Rect {
    Rect::from_size(w, h)
}

fn ash_rect(c: &Ctx, vent: (f32, f32)) -> Rect {
    Rect::new(vent.0 as i32 - 130, vent.1 as i32 - 400, vent.0 as i32 + 210, vent.1 as i32 + 40)
        .intersect(&Rect::new(0, 0, c.w, c.h))
}

// ---------------------------------------------------------------------------
// Damage: where the moving parts are this frame (D71)
// ---------------------------------------------------------------------------

fn rc(cx: f32, cy: f32, l: f32, t: f32, r: f32, b: f32) -> Rect {
    Rect::new((cx - l) as i32, (cy - t) as i32, (cx + r) as i32 + 1, (cy + b) as i32 + 1)
}

/// The four bands of an expanding ring's annulus.
fn ring_rects(centre: (f32, f32), r_tiles: f32, out: &mut Vec<Rect>) {
    let rx = r_tiles * map::iso_consts::HALF_W as f32 * std::f32::consts::SQRT_2;
    let ry = r_tiles * map::iso_consts::HALF_H as f32 * std::f32::consts::SQRT_2;
    let th = 30.0;
    out.push(rc(centre.0, centre.1 - ry, rx + th, th + 12.0, rx + th, th));
    out.push(rc(centre.0, centre.1 + ry, rx + th, th, rx + th, th));
    out.push(rc(centre.0 - rx, centre.1, th, ry + th, th, ry + th));
    out.push(rc(centre.0 + rx, centre.1, th, ry + th, th, ry + th));
}

fn bbox_of(c: &Ctx, pred: impl Fn(f32) -> bool, l: f32, t: f32, r: f32, b: f32) -> Option<Rect> {
    let mut bb: Option<Rect> = None;
    for &(x, y, thr) in c.tiles {
        if !pred(thr) {
            continue;
        }
        let (cx, cy) = tile_centre(c, x, y);
        let q = rc(cx, cy, l, t, r, b);
        bb = Some(match bb {
            Some(o) => o.union(&q),
            None => q,
        });
    }
    bb
}

/// Screen rectangles that change this frame (everything that moves), clipped
/// to the frame. `slow` is true on the few ticks a second that the slowly
/// pulsing things (lava glow, foam, the machine traces) are redrawn.
pub fn damage_rects(c: &Ctx, slow: bool) -> Vec<Rect> {
    let fx = c.fx;
    let u = fx.u;
    let mut v: Vec<Rect> = Vec::new();
    let full = Rect::new(0, 0, c.w, c.h);
    let stride = smoke_stride(c).max(1);
    for (i, &(x, y, code, z)) in c.marks.iter().enumerate() {
        if i % stride != 0 {
            continue;
        }
        let (sx, sy) = c.cam.project(x, y, z as i32);
        if code == 1 {
            v.push(rc(sx, sy, 30.0, 120.0, 50.0, 20.0));
        } else if code == 20 {
            v.push(rc(sx, sy, 60.0, 280.0, 110.0, 30.0));
        } else if code == 7 {
        } else {
            v.push(rc(sx, sy, 30.0, 60.0, 40.0, 10.0));
        }
    }
    match fx.kind {
        kind::METEOR | kind::MOON => {
            let site = ground_screen(c, fx.site.0, fx.site.1);
            let hit = if fx.kind == kind::METEOR { 1.36 } else { 1.7 };
            if fx.kind == kind::METEOR && (1.0..hit).contains(&u) {
                let t = (u - 1.0) / (hit - 1.0);
                let start = (site.0 - c.w as f32 * 0.48, -80.0f32);
                let e = |t: f32| t * t;
                let h = (start.0 + (site.0 - start.0) * e(t), start.1 + (site.1 - start.1) * e(t));
                let tk = (t - 0.25).max(0.0);
                let b = (start.0 + (site.0 - start.0) * e(tk), start.1 + (site.1 - start.1) * e(tk));
                v.push(Rect::new(
                    h.0.min(b.0) as i32 - 120,
                    h.1.min(b.1) as i32 - 120,
                    h.0.max(b.0) as i32 + 120,
                    h.1.max(b.1) as i32 + 120,
                ));
            } else if u >= hit && u < 3.0 {
                let secs = (u - hit).max(0.0) * fx.dur.max(1.0);
                let (flash_s, speed, rmax) = if fx.kind == kind::METEOR { (22.0, 0.05, 26.0) } else { (60.0, 0.08, 46.0) };
                if secs < flash_s {
                    v.push(full);
                } else {
                    let r = (secs * speed).min(rmax);
                    if r < rmax - 1.0 {
                        ring_rects(site, r, &mut v);
                    }
                    v.push(rc(site.0, site.1, 160.0, 380.0, 160.0, 110.0));
                    if secs < 160.0 && fx.kind == kind::METEOR {
                        v.push(rc(site.0, site.1, 460.0, 440.0, 460.0, 60.0));
                    }
                }
            } else if fx.kind == kind::MOON && u >= 1.0 {
                // B61e: the embers are eighty 2-pixel sparks scattered over the
                // picture, not a full-frame effect. Each one damages its own 3x3
                // box (the old positions are unioned in by the caller), and the
                // frame only goes full while the impact's flash is actually on.
                let embers = moon_ember_positions(c);
                if embers.is_empty() {
                    let secs = (u - 1.7).max(0.0) * fx.dur.max(1.0);
                    if (1.0 - ramp(secs, 0.0, 60.0)).powf(1.5) > 0.01 {
                        v.push(full); // the flash over everything
                    }
                    v.push(rc(site.0, site.1, 520.0, 520.0, 520.0, 340.0));
                }
                for (x, y) in embers {
                    v.push(Rect::new(x - 1, y - 2, x + 2, y + 1));
                }
            }
        }
        kind::VOLCANO => {
            let vent = vent_screen(c);
            if u < 1.0 {
                v.push(rc(vent.0, vent.1, 80.0, 190.0, 90.0, 40.0));
            } else {
                // The column is narrow low down and spreads at the top: two
                // rectangles, not one box.
                v.push(Rect::new(vent.0 as i32 - 70, vent.1 as i32 - 210, vent.0 as i32 + 120, vent.1 as i32 + 40));
                v.push(Rect::new(vent.0 as i32 - 130, vent.1 as i32 - 400, vent.0 as i32 + 210, vent.1 as i32 - 200));

            }
        }
        kind::KAIJU => {
            let mut gs = endgeo::giants(&fx.aux, u);
            if u > 0.45 && u < 0.82 {
                gs.push(endgeo::Giant { x: fx.aux[0], y: fx.aux[1], dir: 0, head: (1.0, 0.0), walked: 0.0, mode: endgeo::GiantMode::Walk, fall: 0.0, which: 0 });
            }
            for g in gs {
                let f = ground_screen(c, g.x, g.y);
                v.push(rc(f.0, f.1, 130.0, 150.0, 130.0, 24.0));
                if g.mode == endgeo::GiantMode::Fight {
                    let m = ground_screen(c, g.x + g.head.0 * 1.8, g.y + g.head.1 * 1.8);
                    v.push(rc(m.0, m.1, 50.0, 100.0, 50.0, 20.0));
                }
            }
        }
        kind::ALIENS => {
            let n = fx.aux[2].max(3.0) as usize;
            for s in endgeo::saucers(fx.site, fx.aux[1], n, fx.seed, u, c.t) {
                let g = ground_screen(c, s.x, s.y);
                let top = g.1 - s.alt * 0.62;
                v.push(Rect::new(g.0 as i32 - 70, top as i32 - 40, g.0 as i32 + 70, g.1 as i32 + 36));
            }
        }
        kind::AI => {
            for &(x, y) in c.drones {
                let (sx, sy) = c.cam.project(x, y, crate::iso::level_px(c.map.surface_level_at(x, y)) + 9);
                v.push(rc(sx, sy, 8.0, 8.0, 8.0, 8.0));
            }
            if slow {
                let front = 0.12 + ramp(u, 1.0, 1.92) * 0.88;
                if let Some(b) = bbox_of(c, |t| t <= front, 34.0, 18.0, 34.0, 18.0) {
                    v.push(b);
                }
            }
        }
        kind::FLOOD => {
            if slow {
                let fr = fx.frac;
                if let Some(b) = bbox_of(c, |t| (-0.02..0.09).contains(&(fr - t)), 36.0, 40.0, 36.0, 20.0) {
                    v.push(b);
                }
            }
        }
        kind::SUNSET => {
            let (sx, sy) = ground_screen(c, fx.site.0, fx.site.1);
            v.push(Rect::new(sx as i32 - 260, sy as i32 - 520, sx as i32 + 260, sy as i32 + 60));
        }
        _ => {}
    }
    v.into_iter().map(|r| r.intersect(&full)).filter(|r| !r.is_empty()).collect()
}

/// The part of the frame the ending's ground overlays can touch (lava, ice,
/// traces, foam, the cone, the haze bands): a rectangle test per composite
/// call keeps a damaged rectangle elsewhere (a walking person) from paying for
/// the overlay's loops.
pub fn ground_bounds(c: &Ctx) -> Rect {
    let full = Rect::new(0, 0, c.w, c.h);
    let fx = c.fx;
    match fx.kind {
        kind::SMOG | kind::SUNSET => full,
        kind::VOLCANO | kind::ICE_AGE | kind::AI | kind::FLOOD => {
            let mut bb: Option<Rect> = None;
            for &(x, y, _) in c.tiles {
                let (cx, cy) = tile_centre(c, x, y);
                let q = rc(cx, cy, 40.0, 24.0, 40.0, 24.0);
                bb = Some(match bb {
                    Some(o) => o.union(&q),
                    None => q,
                });
            }
            if fx.kind == kind::VOLCANO {
                let (cx, cy) = c.cam.project(fx.site.0, fx.site.1, 0);
                let q = rc(cx, cy, 340.0, 220.0, 340.0, 220.0);
                bb = Some(match bb {
                    Some(o) => o.union(&q),
                    None => q,
                });
            }
            bb.map(|b| b.intersect(&full)).unwrap_or(Rect::EMPTY)
        }
        _ => Rect::EMPTY,
    }
}

/// Everything `draw_world` can put on the frame this tick: the marks and the
/// damage rectangles.
pub fn world_bounds(c: &Ctx, damage: &[Rect]) -> Rect {
    let full = Rect::new(0, 0, c.w, c.h);
    let mut bb: Option<Rect> = None;
    let mut add = |q: Rect| {
        bb = Some(match bb {
            Some(o) => o.union(&q),
            None => q,
        });
    };
    for &(x, y, _code, z) in c.marks {
        let (sx, sy) = c.cam.project(x, y, z as i32);
        add(rc(sx, sy, 80.0, 300.0, 110.0, 40.0));
    }
    for r in damage {
        add(*r);
    }
    if matches!(c.fx.kind, kind::SUNSET | kind::MOON) {
        return full;
    }
    bb.map(|b| b.intersect(&full)).unwrap_or(Rect::EMPTY)
}
