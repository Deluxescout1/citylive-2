//! The world's light (D74): one [`LightState`] per frame, and the shadows it
//! casts.
//!
//! Everything that is lit reads this module: the ambient multiply baked into
//! the static layer (B13, still the [`lut`] below), the window glow (D25), the
//! cast shadows in [`crate::shadow`], and the small projected shadows that
//! moving things draw for themselves.
//!
//! ## The convention everything shares
//!
//! The sky band (`sky::draw`) paints the panorama the camera faces: **south at
//! the centre of the band, east at the left, west at the right**, north behind
//! the viewer. The ground therefore runs, in tile space,
//!
//! * south = the far edge (up-screen), north = the near edge (down-screen),
//! * east = screen left, west = screen right.
//!
//! With that, a body at azimuth `az` and altitude `alt` sits in the direction
//! `toward(az) = ((cos az − sin az), (sin az + cos az)) / √2` (tiles) and a
//! shadow of an object `h` px tall reaches `h / (tan(alt) · TILE_W)` tiles the
//! opposite way — long at dawn, short at noon, gone when the sun is down. That
//! is the whole model: no light is painted anywhere except from here.

use crate::framebuffer::Framebuffer;
use crate::iso::Camera;
use crate::palette::{self, Color, TRANSPARENT};
use map::iso_consts::TILE_W;

/// 256-entry lookup table per channel, so tinting a frame is three table reads
/// per pixel instead of a float multiply.
pub type Lut = [[u8; 256]; 3];

pub fn lut(tint: [f32; 3]) -> Box<Lut> {
    let mut l = Box::new([[0u8; 256]; 3]);
    for c in 0..3 {
        for v in 0..256 {
            let out = v as f32 * tint[c];
            l[c][v] = out.clamp(0.0, 255.0) as u8;
        }
    }
    l
}

#[inline]
pub fn apply(p: Color, lut: &Lut) -> Color {
    if p == TRANSPARENT {
        return TRANSPARENT;
    }
    palette::with_alpha(
        palette::rgb(
            lut[0][palette::r(p) as usize],
            lut[1][palette::g(p) as usize],
            lut[2][palette::b(p) as usize],
        ),
        palette::alpha(p),
    )
}

/// Convenience for sprites: tint every pixel of a slice in place.
pub fn tint_pixels(px: &mut [Color], lut: &Lut) {
    for p in px.iter_mut() {
        *p = apply(*p, lut);
    }
}

// ---------------------------------------------------------------------------
// The light state
// ---------------------------------------------------------------------------

/// A body in the sky: altitude above the horizon and azimuth from true north,
/// both in degrees (the frame `world::sun` / `world::moon` hand out).
#[derive(Clone, Copy, Debug)]
pub struct Orb {
    pub alt: f32,
    pub az: f32,
}

impl Orb {
    /// A body that is not up at all (the default: no sunlight, no moonlight).
    pub const DOWN: Orb = Orb {
        alt: -90.0,
        az: 0.0,
    };

    pub fn new(alt: f64, az: f64) -> Self {
        Self {
            alt: alt as f32,
            az: az as f32,
        }
    }

    /// Is it above the horizon by `deg` degrees? (A grazing body lights nothing
    /// at pixel scale, so the tests use a small threshold.)
    pub fn up(&self, deg: f32) -> bool {
        self.alt > deg
    }
}

impl Default for Orb {
    fn default() -> Self {
        Orb::DOWN
    }
}

/// Everything the world's lighting depends on, for one instant.
///
/// The caller (the wallpaper's sky pass) fills it from the `sky` crate: the
/// palette gives `ambient` / `glow` / `dir_strength`, the weather gives
/// `cloud`, and `sun` / `moon` come straight from `world::sun` and
/// `world::moon`.
#[derive(Clone, Copy, Debug)]
pub struct LightState {
    pub sun: Orb,
    pub moon: Orb,
    /// Illuminated fraction of the moon, 0..=1 (`moon::Phase::illumination`).
    pub moon_illum: f32,
    /// Per-channel multiply for the ambient light (the sky palette's tint,
    /// with the weather folded in).
    pub ambient: [f32; 3],
    /// 0..=1 night window lighting (D25).
    pub glow: f32,
    /// 0..=1 cloud cover: overcast softens and weakens the cast shadows.
    pub cloud: f32,
    /// 0..=1 strength of the main directional light (`light_dir_strength`,
    /// which already includes the moonlight term the palette adds).
    pub dir_strength: f32,
    /// Local hour of the day, 0..24 (B58b item 7): who has the lights on is a
    /// question about the *clock*, not just the light level — an office block at
    /// 02:00 and a house at 02:00 are both dark, but at 19:00 the offices are
    /// emptying and the houses are full.
    pub hour: f32,
}

impl Default for LightState {
    fn default() -> Self {
        Self {
            sun: Orb::DOWN,
            moon: Orb::DOWN,
            moon_illum: 0.0,
            ambient: [1.0, 1.0, 1.0],
            glow: 0.0,
            cloud: 0.0,
            dir_strength: 0.0,
            hour: 12.0,
        }
    }
}

impl LightState {
    /// Build from the astronomy the sky crate hands out.
    pub fn from_sky(sun_alt: f64, sun_az: f64, moon_alt: f64, moon_az: f64, moon_illum: f64) -> Self {
        Self {
            sun: Orb::new(sun_alt, sun_az),
            moon: Orb::new(moon_alt, moon_az),
            moon_illum: moon_illum as f32,
            ..Self::default()
        }
    }

    /// The body that casts the world's shadows right now, if any.
    ///
    /// The sun whenever it is up; otherwise the moon, but only when it is
    /// genuinely bright — a crescent gives no readable shadow at pixel scale,
    /// and D74 wants "none when the sun is down unless the moon is bright".
    pub fn caster(&self) -> Option<Caster> {
        if self.sun.up(0.5) {
            let strength = (self.dir_strength * (1.0 - 0.72 * self.cloud.clamp(0.0, 1.0)))
                .clamp(0.0, 1.0);
            return Some(Caster {
                alt: self.sun.alt,
                toward: toward(self.sun.az),
                strength,
                moon: false,
            });
        }
        // Night: moonlight scales with how full the moon is, and a low moon
        // throws long faint ones. Below a third lit it is invisible anyway.
        let lit = (self.moon_illum - 0.28) / 0.72;
        if self.moon.up(4.0) && lit > 0.0 {
            let strength = (0.34 * lit * (1.0 - 0.8 * self.cloud.clamp(0.0, 1.0)))
                .clamp(0.0, 1.0);
            if strength > 0.01 {
                return Some(Caster {
                    alt: self.moon.alt,
                    toward: toward(self.moon.az),
                    strength,
                    moon: true,
                });
            }
        }
        None
    }

    /// The body that casts **cloud** shadows (B57). Clouds are up in the air and
    /// their shadow reads as weather rather than as shade, so the rule is
    /// stricter than [`Self::caster`]'s (D72): the sun, or a moon that is both
    /// more than half full and well up — a low or thin moon washes a patch of
    /// ground without anyone noticing, and painting it would only cost frames.
    pub fn cloud_caster(&self) -> Option<Caster> {
        let c = self.caster()?;
        if c.moon && !(self.moon_illum > 0.60 && self.moon.alt > 15.0) {
            return None;
        }
        Some(c)
    }

    /// Per-channel multiplier of a surface in full shadow: only the sky lights
    /// it, so it goes down *and* cool (the sun's warmth is what is missing).
    ///
    /// How much of this a frame actually gets is [`Caster::strength`], which
    /// falls with a low sun, cloud and a thin moon — so this is the noon value.
    pub fn shadow_mult(&self) -> [f32; 3] {
        // A sunlit surface is sun + sky; a shadowed one is sky only. Skylight is
        // maybe a sixth of a clear noon and far bluer — pressed into the range
        // pixel art reads as "shade" rather than "hole" (D4's moody blue).
        [0.58, 0.62, 0.72]
    }

    /// The air colour a lit surface keeps, for the sun's own warmth. Handy for
    /// the light pools and for report screenshots.
    pub fn sun_colour(&self) -> Color {
        // 0° = deep orange, 10° = gold, 30°+ = white.
        let t = (self.sun.alt / 30.0).clamp(0.0, 1.0);
        let warm = palette::rgb(0xF2, 0xA2, 0x5E);
        let white = palette::rgb(0xFF, 0xF6, 0xE8);
        palette::mix(warm, white, t)
    }

    /// The ground offset a shadow of a thing `height_px` tall gets, in tiles
    /// (negative = away from the light). `None` when nothing is casting.
    pub fn ground_offset(&self, height_px: f32) -> Option<(f32, f32)> {
        let c = self.caster()?;
        if c.strength <= 0.01 {
            return None;
        }
        let alt = c.alt.max(1.0).to_radians();
        let len_tiles = height_px / (alt.tan() * TILE_W as f32);
        Some((-c.toward.0 * len_tiles, -c.toward.1 * len_tiles))
    }
}

/// The active shadow-casting body.
#[derive(Clone, Copy, Debug)]
pub struct Caster {
    pub alt: f32,
    /// Unit ground direction *toward* the body, in tile space.
    pub toward: (f32, f32),
    /// 0..=1 how dark the shadow reads (cloud, low sun and moon phase).
    pub strength: f32,
    pub moon: bool,
}

/// Unit ground vector toward a body at azimuth `az` (see the module docs).
pub fn toward(az_deg: f32) -> (f32, f32) {
    let r = az_deg.to_radians();
    let (s, c) = (r.sin(), r.cos());
    let inv = std::f32::consts::FRAC_1_SQRT_2;
    ((c - s) * inv, (s + c) * inv)
}

// ---------------------------------------------------------------------------
// The shadow multiply
// ---------------------------------------------------------------------------

/// How many steps the shadow strength is quantised to (keeps the per-pixel
/// lookup a small table instead of three multiplies).
pub const SHADOW_STEPS: usize = 16;

/// Per-channel multiply for every shadow strength: `tint[step][channel][v]`.
/// Built once per light change, read three times per shadowed pixel.
#[derive(Clone)]
pub struct ShadowLut {
    pub t: Box<[[[u8; 256]; 3]; SHADOW_STEPS + 1]>,
}

impl ShadowLut {
    pub fn new(mult: [f32; 3], strength: f32) -> Box<Self> {
        let strength = strength.clamp(0.0, 1.0);
        let mut t: Box<[[[u8; 256]; 3]; SHADOW_STEPS + 1]> =
            Box::new([[[0u8; 256]; 3]; SHADOW_STEPS + 1]);
        for step in 0..=SHADOW_STEPS {
            let s = step as f32 / SHADOW_STEPS as f32 * strength;
            for c in 0..3 {
                let f = 1.0 + (mult[c] - 1.0) * s;
                for v in 0..256 {
                    t[step][c][v] = (v as f32 * f).clamp(0.0, 255.0) as u8;
                }
            }
        }
        Box::new(ShadowLut { t })
    }

    #[inline]
    pub fn apply(&self, c: Color, coverage: u8) -> Color {
        if coverage == 0 || c == TRANSPARENT {
            return c;
        }
        let step = ((coverage as usize * SHADOW_STEPS + 127) / 255).min(SHADOW_STEPS);
        palette::with_alpha(
            palette::rgb(
                self.t[step][0][palette::r(c) as usize],
                self.t[step][1][palette::g(c) as usize],
                self.t[step][2][palette::b(c) as usize],
            ),
            palette::alpha(c),
        )
    }

    /// No shadow at all (the state when nothing is casting).
    pub fn none() -> Box<Self> {
        Self::new([1.0; 3], 0.0)
    }
}

// ---------------------------------------------------------------------------
// Shadows of things that move
// ---------------------------------------------------------------------------

/// A small projected shadow drawn under a moving thing every frame (D74 item 3).
/// `dx, dy` are the tiles the shadow's far end sits from the thing's feet per
/// pixel of its height; `cov` is its strength (the static shadows' own, so a
/// person's shade is as dark as the wall's next to it) and `mult` the per
/// channel multiply of a fully shadowed surface: a darker, cooler version of
/// whatever it falls on, never a flat grey overlay (B68).
#[derive(Clone, Copy, Debug)]
pub struct ActorShadow {
    pub dx: f32,
    pub dy: f32,
    pub cov: f32,
    pub mult: [f32; 3],
}

impl LightState {
    /// The per-frame shadow a car, a person or an animal draws, or `None` when
    /// nothing is casting (night with no moon, or dead overcast).
    pub fn actor(&self) -> Option<ActorShadow> {
        let c = self.caster()?;
        let cov = c.strength;
        if cov < 0.04 {
            return None;
        }
        let alt = c.alt.max(2.0).to_radians();
        let per_px = -1.0 / (alt.tan() * TILE_W as f32);
        Some(ActorShadow {
            dx: c.toward.0 * per_px,
            dy: c.toward.1 * per_px,
            cov,
            mult: self.shadow_mult(),
        })
    }
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Draw the flat shadow of a thing standing at `(x, y, z)` with ground half
/// extents `hw` x `hh` tiles and `height_px` of height. The shadow is exactly
/// the footprint rectangle swept along the light (a hexagon in tile space,
/// B68), multiplied into whatever is below so it keeps the ground's own colour,
/// depth-tested so a shadow behind a wall stays behind it, and left out where
/// the static shadow plane already shades the ground.
#[allow(clippy::too_many_arguments)]
pub fn draw_actor_shadow(
    fb: &mut Framebuffer,
    cam: &Camera,
    depth: &[u16],
    key: u16,
    x: f32,
    y: f32,
    z: i32,
    hw: f32,
    hh: f32,
    height_px: f32,
    s: &ActorShadow,
    test_depth: bool,
) -> crate::Rect {
    draw_actor_shadow_in(fb, cam, depth, &[], key, x, y, z, hw, hh, height_px, s, test_depth)
}

/// B101: the pixels `px0..px1` of the row at ground-diagonal `v` that lie in the
/// swept footprint of a `hw` x `hh` rectangle centred on `(x, y)` moved along
/// `(ox, oy)`, clipped to `cx0..cx1`; `None` when the row misses it.
///
/// In ground coordinates a pixel row is the line `X + Y = v`. Writing `s` for
/// `v - x - y`, a point on it with offset `r` along X is in the footprint when
/// some `t` in `0..=1` has `|r - t ox| <= hw` and `|s - r - t oy| <= hh`. Those
/// bound `t` (so the row can miss) and, for the `t` that are left, bound `r` by
/// two piecewise-linear functions whose extremes sit at the ends of the `t`
/// range or at one corner each. Nothing is searched per pixel.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn row_run(
    x: f32,
    y: f32,
    v: f32,
    ox: f32,
    oy: f32,
    hw: f32,
    hh: f32,
    cam_ox: f32,
    cx0: i32,
    cx1: i32,
) -> Option<(i32, i32)> {
    use map::iso_consts::HALF_W;
    let s = v - x - y;
    let k = ox + oy;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    if k.abs() < 1e-6 {
        if s.abs() > hw + hh {
            return None;
        }
    } else {
        let (a, b) = ((s - hw - hh) / k, (s + hw + hh) / k);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
        if t0 > t1 {
            return None;
        }
    }
    let lo_at = |t: f32| (t * ox - hw).max(s - t * oy - hh);
    let hi_at = |t: f32| (t * ox + hw).min(s - t * oy + hh);
    let (mut lo, mut hi) = (lo_at(t0).min(lo_at(t1)), hi_at(t0).max(hi_at(t1)));
    if k.abs() >= 1e-6 {
        // Where the two lower (and the two upper) bounds cross.
        let tl = (s - hh + hw) / k;
        if tl > t0 && tl < t1 {
            lo = lo.min(lo_at(tl));
        }
        let tu = (s + hh - hw) / k;
        if tu > t0 && tu < t1 {
            hi = hi.max(hi_at(tu));
        }
    }
    if lo > hi {
        return None;
    }
    // u = X - Y = 2r + (x - y - s); pixel centre px + 0.5 sits at u * HALF_W + cam_ox.
    let base = x - y - s;
    let e = 1e-3;
    let a = (((2.0 * lo + base) * HALF_W as f32 + cam_ox - 0.5) - e).ceil() as i32;
    let b = (((2.0 * hi + base) * HALF_W as f32 + cam_ox - 0.5) + e).floor() as i32 + 1;
    let (a, b) = (a.max(cx0), b.min(cx1));
    (a < b).then_some((a, b))
}

/// As [`draw_actor_shadow`], with the frame's static shadow plane so a figure
/// standing in a wall's shade is not darkened twice.
#[allow(clippy::too_many_arguments)]
pub fn draw_actor_shadow_in(
    fb: &mut Framebuffer,
    cam: &Camera,
    depth: &[u16],
    plane: &[u8],
    key: u16,
    x: f32,
    y: f32,
    z: i32,
    hw: f32,
    hh: f32,
    height_px: f32,
    s: &ActorShadow,
    test_depth: bool,
) -> crate::Rect {
    use map::iso_consts::{HALF_H, HALF_W};
    let (ox, oy) = (s.dx * height_px, s.dy * height_px);
    // At least a pixel wide, or a person is a dotted line.
    let hw = hw.max(0.5 / TILE_W as f32 * 1.2);
    let hh = hh.max(0.5 / TILE_W as f32 * 1.2);
    // Screen bounds of the swept footprint (its eight corners).
    let mut bounds = crate::Rect::EMPTY;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for t in [0.0f32, 1.0] {
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let c = cam.project(x + sx * hw + ox * t, y + sy * hh + oy * t, z);
            x0 = x0.min(c.0 as i32 - 1);
            x1 = x1.max(c.0 as i32 + 1);
            y0 = y0.min(c.1 as i32 - 1);
            y1 = y1.max(c.1 as i32 + 1);
        }
    }
    let r = crate::Rect::new(x0, y0, x1 + 1, y1 + 1);
    bounds = if bounds.is_empty() { r } else { bounds.union(&r) };
    let clip = fb.clip().intersect(&r);
    if clip.is_empty() {
        return bounds;
    }
    SCRATCH.with(|sc| {
        let mut hit = sc.borrow_mut();
        hit.clear();
        for py in clip.y0..clip.y1 {
            let v = (py as f32 + 0.5 - cam.oy as f32 + z as f32) / HALF_H as f32;
            // B101: the swept footprint is a convex hexagon on the ground, so a
            // pixel row (a diagonal line on the ground) crosses it in one run;
            // find the run instead of testing every pixel of the box.
            let Some((px0, px1)) = row_run(x, y, v, ox, oy, hw, hh, cam.ox as f32, clip.x0, clip.x1) else {
                continue;
            };
            for px in px0..px1 {
                let i = py as usize * fb.w + px as usize;
                if test_depth && !depth.is_empty() {
                    // Ground never hides a shadow (its own painter key changes
                    // from tile to tile); only something *built* in front does,
                    // i.e. an even key on a nearer diagonal.
                    let d = depth[i];
                    if d != 0 && d & 1 == 0 && d > key {
                        continue;
                    }
                }
                hit.push(i as u32);
            }
        }
        let cov = s.cov.clamp(0.0, 1.0);
        let f = [
            1.0 + (s.mult[0] - 1.0) * cov,
            1.0 + (s.mult[1] - 1.0) * cov,
            1.0 + (s.mult[2] - 1.0) * cov,
        ];
        for &i in hit.iter() {
            let i = i as usize;
            let c = fb.px()[i];
            if c == TRANSPARENT {
                continue;
            }
            // Already in the static shade: only what that leaves is ours.
            let left = if plane.len() == fb.w * fb.h { 1.0 - plane[i] as f32 / 255.0 } else { 1.0 };
            let g = |k: usize| 1.0 + (f[k] - 1.0) * left;
            let n = palette::with_alpha(
                palette::rgb(
                    (palette::r(c) as f32 * g(0)) as u8,
                    (palette::g(c) as f32 * g(1)) as u8,
                    (palette::b(c) as f32 * g(2)) as u8,
                ),
                palette::alpha(c),
            );
            fb.px_mut()[i] = n;
        }
    });
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B101: the per-row run finds the same pixels as testing every pixel of the
    /// box with the swept-footprint inequalities (the way it used to be done).
    #[test]
    fn the_row_run_matches_the_per_pixel_test() {
        use map::iso_consts::{HALF_H, HALF_W};
        let mut seed = 0x1234_5678u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut worst = 0usize;
        let mut total = 0usize;
        for _ in 0..4000 {
            let (x, y) = (20.0 + rnd() * 40.0, 20.0 + rnd() * 40.0);
            let (hw, hh) = (0.01 + rnd() * 0.4, 0.01 + rnd() * 0.4);
            let (ox, oy) = ((rnd() - 0.5) * 3.0, (rnd() - 0.5) * 3.0);
            let (cam_ox, cam_oy, z) = (3000.0 + rnd() * 10.0, 200.0 + rnd() * 10.0, (rnd() * 40.0) as i32);
            let span = |q: f32, o: f32, h: f32| -> (f32, f32) {
                if o.abs() < 1e-5 {
                    if q.abs() <= h { (0.0, 1.0) } else { (1.0, 0.0) }
                } else {
                    let (a, b) = ((q - h) / o, (q + h) / o);
                    (a.min(b).max(0.0), a.max(b).min(1.0))
                }
            };
            // The screen box of the footprint.
            let (px_lo, px_hi) = (((x - y) * HALF_W as f32 + cam_ox) as i32 - 120, ((x - y) * HALF_W as f32 + cam_ox) as i32 + 120);
            let cy = ((x + y) * HALF_H as f32 + cam_oy - z as f32) as i32;
            for py in (cy - 80)..(cy + 80) {
                let v = (py as f32 + 0.5 - cam_oy + z as f32) / HALF_H as f32;
                let run = row_run(x, y, v, ox, oy, hw, hh, cam_ox, px_lo, px_hi);
                for px in px_lo..px_hi {
                    let u = (px as f32 + 0.5 - cam_ox) / HALF_W as f32;
                    let (tx, ty) = ((u + v) * 0.5 - x, (v - u) * 0.5 - y);
                    let (a0, a1) = span(tx, ox, hw);
                    let (b0, b1) = span(ty, oy, hh);
                    let brute = a0.max(b0) <= a1.min(b1);
                    let fast = run.is_some_and(|(a, b)| px >= a && px < b);
                    total += brute as usize;
                    if brute != fast {
                        worst += 1;
                    }
                }
            }
        }
        // Only razor-edge pixels (inside the 1e-3 slack) may differ.
        assert!(worst * 5000 <= total, "{worst} of {total} pixels differ");
    }

    /// The azimuth → ground direction map has to agree with the sky band:
    /// south (180°) is the far edge (up-screen), east (90°) is screen left.
    #[test]
    fn azimuth_maps_onto_the_screen_like_the_sky_band() {
        // screen u = (x - y), screen v = (x + y): up-screen is negative v.
        let screen = |az: f32| {
            let (x, y) = toward(az);
            (x - y, x + y)
        };
        let south = screen(180.0);
        assert!(south.0.abs() < 1e-4, "south must not lean sideways: {south:?}");
        assert!(south.1 < -1.0, "south must be up-screen: {south:?}");
        let east = screen(90.0);
        assert!(east.0 < -1.0, "east must be screen left: {east:?}");
        assert!(east.1.abs() < 1e-4, "east must not lean up or down: {east:?}");
        let west = screen(270.0);
        assert!(west.0 > 1.0, "west must be screen right: {west:?}");
        let north = screen(0.0);
        assert!(north.1 > 1.0, "north must be down-screen: {north:?}");
        // Unit length everywhere, so a shadow's length only depends on altitude.
        for az in (0..360).step_by(7) {
            let (x, y) = toward(az as f32);
            assert!((x * x + y * y - 1.0).abs() < 1e-3, "az {az} not unit");
        }
    }

    /// Shadow length follows 1/tan(altitude) and flips with the azimuth.
    #[test]
    fn shadow_length_and_direction_follow_the_sun() {
        let mut l = LightState::default();
        l.dir_strength = 1.0;
        l.sun = Orb::new(45.0, 180.0);
        let (dx, dy) = l.ground_offset(TILE_W as f32).unwrap();
        // A body one tile tall at 45° casts one tile of shadow, away from south
        // (down-screen), i.e. north = (+1, +1)/√2.
        assert!((dx - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "dx {dx}");
        assert!((dy - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "dy {dy}");

        // Low sun: much longer; high sun: much shorter.
        let mut low = l;
        low.sun = Orb::new(10.0, 180.0);
        let (lx, ly) = low.ground_offset(TILE_W as f32).unwrap();
        let len_low = (lx * lx + ly * ly).sqrt();
        let len_45 = (dx * dx + dy * dy).sqrt();
        assert!(len_low > 5.0 * len_45, "10° shadow {len_low} vs 45° {len_45}");
        let mut high = l;
        high.sun = Orb::new(80.0, 180.0);
        let (hx, hy) = high.ground_offset(TILE_W as f32).unwrap();
        assert!((hx * hx + hy * hy).sqrt() < 0.3 * len_45);

        // Morning: the sun is in the east (screen left), so the shadow goes
        // right (screen +u).
        let mut morning = l;
        morning.sun = Orb::new(30.0, 90.0);
        let (mx, my) = morning.ground_offset(TILE_W as f32).unwrap();
        assert!(mx - my > 0.0, "morning shadow must point screen-right");
    }

    /// No sun, no moon → no cast shadow; a full moon at night → faint ones.
    #[test]
    fn night_needs_a_bright_moon() {
        let mut l = LightState::default();
        l.sun = Orb::new(-20.0, 180.0);
        l.moon = Orb::new(40.0, 120.0);
        l.moon_illum = 0.1;
        l.dir_strength = 0.3 * 0.1;
        assert!(l.caster().is_none(), "a crescent must cast nothing");

        l.moon_illum = 1.0;
        l.dir_strength = 0.3;
        let c = l.caster().expect("a full moon must cast");
        assert!(c.moon);
        assert!(c.strength < 0.4, "moonlight shadows stay faint: {}", c.strength);
        let day = LightState {
            sun: Orb::new(30.0, 180.0),
            dir_strength: 1.0,
            ..LightState::default()
        };
        assert!(day.caster().unwrap().strength > 2.0 * c.strength);
    }

    /// Overcast softens the shadows away; a sun below the horizon casts none.
    #[test]
    fn overcast_weakening_and_no_sun() {
        let clear = LightState {
            sun: Orb::new(40.0, 180.0),
            dir_strength: 1.0,
            cloud: 0.0,
            ..LightState::default()
        };
        let grey = LightState { cloud: 1.0, ..clear };
        assert!(grey.caster().unwrap().strength < 0.35 * clear.caster().unwrap().strength);

        let mut down = clear;
        down.sun = Orb::new(-2.0, 180.0);
        down.moon = Orb::DOWN;
        assert!(down.caster().is_none());
        assert!(down.ground_offset(40.0).is_none());
        assert!(down.actor().is_none());
    }

    /// The shadow multiply darkens (and cools) without leaving its range.
    #[test]
    fn shadow_lut_darkens_and_keeps_channels_in_range() {
        let l = LightState {
            sun: Orb::new(30.0, 180.0),
            dir_strength: 1.0,
            ..LightState::default()
        };
        let lut = ShadowLut::new(l.shadow_mult(), 1.0);
        let c = palette::rgb(200, 200, 200);
        let dark = lut.apply(c, 255);
        assert!(palette::r(dark) < 200 && palette::b(dark) < 200);
        // Cooler: blue survives more than red.
        assert!(palette::b(dark) > palette::r(dark));
        // Undarkened pixels are untouched.
        assert_eq!(lut.apply(c, 0), c);
        // Monotone in the coverage.
        let mut prev = 255;
        for cov in (0..=255).step_by(8) {
            let r = palette::r(lut.apply(c, cov)) as i32;
            assert!(r <= prev, "shadow must not brighten with coverage");
            prev = r;
        }
    }

    #[test]
    fn warm_at_dawn_cool_at_noon() {
        let mut l = LightState::default();
        l.sun = Orb::new(2.0, 90.0);
        let dawn = l.sun_colour();
        l.sun = Orb::new(60.0, 180.0);
        let noon = l.sun_colour();
        assert!(palette::r(dawn) > palette::b(dawn) + 40, "dawn must be orange");
        assert!(palette::b(noon) > 220, "noon light must be near white");
    }
}
