//! Clouds (D72 / B57 / B57b): real cover, drift and type — in the sky band — and
//! the soft shadows they throw over the land, from the same light model
//! everything else reads (D74 / B60).
//!
//! ## The camera is below the clouds
//!
//! B57 first drew the low field as translucent sprite *boxes* over the city. At
//! this camera they read as fog balls at tree height, and the land showed
//! through them (the B57b review). So the low field is now an **invisible
//! shadow field**: the same clouds, in tile space, drifting with the same wind —
//! but only their shadows are painted, on the ground, the streets, the roofs and
//! the walls. What you see is the sky band; what you feel is the shade crossing
//! the city.
//!
//! ## What a cloud is
//!
//! A pre-rasterised sprite: overlapping lobes (ellipses, optionally rotated)
//! baked once into a [`Shape`] — a coverage byte and a tone index per pixel
//! (`body`, `top`, `base`, `rain`) — plus a **shade** mask: the same coverage
//! box-blurred, which is what a shadow uses. The tone is resolved to a colour at
//! draw time, so one sprite serves every hour.
//!
//! The band has two layers:
//!
//! * the **deck** — a continuous textured lid whose underside undulates and hangs
//!   in darker clumps, drawn when the cover earns it (a flat grey fill was the
//!   other half of the B57b review);
//! * the **cells** — cumulus, stratus sheets, towering storm cells with anvils
//!   and lit edges on the sun's side, and high cirrus wisps.
//!
//! Every sprite's canvas is fitted to its lobes, so nothing is ever cut by a
//! rectangle; the band fades out towards the horizon instead of ending on a line.

use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::Camera;
use crate::light::{LightState, ShadowLut};
use crate::palette::{self, Color, TRANSPARENT};
use map::iso_consts::{HALF_H, HALF_W, TILE_W};
use map::TileMap;

/// Deterministic hash in `0..1` (the backdrop's, so a world's clouds match the
/// rest of its sky).
pub fn hash(i: i64, seed: u64) -> f32 {
    let mut x = (i as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Periodic 1D value noise in `0..1`: `cells` lattice points over [`DECK_LEN`].
fn pn1(x: f32, cells: i32, seed: u64) -> f32 {
    let fx = x * cells as f32 / DECK_LEN as f32;
    let ix = fx.floor() as i32;
    let t = fx - ix as f32;
    let s = t * t * (3.0 - 2.0 * t);
    let at = |d: i32| hash((ix + d).rem_euclid(cells) as i64 * 131, seed);
    at(0) + (at(1) - at(0)) * s
}

/// Periodic 2D value noise in `0..1` over the deck's `DECK_LEN x DECK_TEX_H`
/// texture, `nx x ny` lattice cells: it wraps exactly, so the deck can be scrolled
/// by index without a seam, and `nx >> ny` stretches the grain along the sky.
fn pn2(x: f32, y: f32, nx: i32, ny: i32, seed: u64) -> f32 {
    let (fx, fy) = (x * nx as f32 / DECK_LEN as f32, y * ny as f32 / DECK_TEX_H as f32);
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let at = |dx: i32, dy: i32| {
        let a = (ix + dx).rem_euclid(nx);
        let b = (iy + dy).rem_euclid(ny);
        hash(a as i64 * 131 + b as i64 * 977, seed)
    };
    let v0 = at(0, 0) + (at(1, 0) - at(0, 0)) * sx;
    let v1 = at(0, 1) + (at(1, 1) - at(0, 1)) * sx;
    v0 + (v1 - v0) * sy
}

/// Smooth 2D value noise in `0..1`: the deck's undulating base.
fn vnoise(x: f32, y: f32, seed: u64) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let at = |dx: i64, dy: i64| hash((ix as i64 + dx) * 73 + (iy as i64 + dy) * 151, seed);
    let a = at(0, 0) + (at(1, 0) - at(0, 0)) * sx;
    let b = at(0, 1) + (at(1, 1) - at(0, 1)) * sx;
    a + (b - a) * sy
}

// ---------------------------------------------------------------------------
// Ink: what colour a cloud is at this instant
// ---------------------------------------------------------------------------

/// The sky palette, flattened to what cloud ink needs — so `render` never has to
/// know about `sky::Palette` (the dependency runs the other way).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyTint {
    pub horizon: Color,
    pub zenith: Color,
    pub ambient: [f32; 3],
    /// Sky palette window glow: 0 by day, ~0.9 at dusk (how orange the light is).
    pub glow: f32,
    /// 0 by day, 1 at night (star alpha of the hour).
    pub star_alpha: f32,
    /// Where the sun sits across the band: −1 at the left edge, +1 at the right.
    /// A cloud lights up on that side (D72: "lit edges by the sun direction").
    pub sun_x: f32,
}

impl SkyTint {
    pub const DAY: SkyTint = SkyTint {
        horizon: 0xFFA9D6F0,
        zenith: 0xFF3884D6,
        ambient: [1.0, 1.0, 0.99],
        glow: 0.03,
        star_alpha: 0.0,
        sun_x: 0.0,
    };
}

impl Default for SkyTint {
    fn default() -> Self {
        Self::DAY
    }
}

/// The tones a cloud is painted in, how solid it is, and which way the sun is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    /// The bulk of a cloud.
    pub body: Color,
    /// Sunlit (or moonlit) tops.
    pub top: Color,
    /// Shaded bases — sunset-coloured undersides at dawn and dusk.
    pub base: Color,
    /// The lit rim on the sun's side.
    pub rim: Color,
    /// A storm's rain shaft.
    pub rain: Color,
    /// 0..=1 opacity.
    pub alpha: f32,
    /// −1 sun at the left … +1 sun at the right: the side a cloud is lit from.
    pub lit: f32,
    /// The deck's own tones: a lid is flatter and greyer than a puff.
    pub deck: Color,
    pub deck_base: Color,
    /// B90: the sun-side glow of dusk: how strong it is (0 by day and night, 1 at
    /// sunset), the bright colour on edges and tops and the deep colour on bases.
    pub warm_k: f32,
    pub warm_edge: Color,
    pub warm_base: Color,
}

impl Default for Ink {
    fn default() -> Self {
        Ink {
            body: palette::rgb(0xEE, 0xEE, 0xF4),
            top: palette::rgb(0xFF, 0xFF, 0xFF),
            base: palette::rgb(0xA8, 0xB2, 0xC6),
            rim: palette::rgb(0xFF, 0xF6, 0xE8),
            rain: palette::rgb(0x78, 0x84, 0x9A),
            alpha: 0.90,
            lit: 0.0,
            deck: palette::rgb(0xD6, 0xDA, 0xDF),
            deck_base: palette::rgb(0x9A, 0xA2, 0xAE),
            warm_k: 0.0,
            warm_edge: palette::rgb(0xFF, 0xD8, 0xA8),
            warm_base: palette::rgb(0xE0, 0x80, 0x60),
        }
    }
}

impl Ink {
    /// The hour's cloud colour (D4/D72): white with lit tops and blue-grey bases
    /// by day, sunset-coloured undersides at dawn and dusk, dim silver by
    /// moonlight, and nearly a silhouette on a dark night.
    ///
    /// B67: the day and the night inks used to switch on `star_alpha > 0.35`, so
    /// every cloud in the sky changed colour in one step once a dusk and once a
    /// dawn. Both are built and blended across the twilight instead.
    pub fn new(t: SkyTint, light: &LightState, cover: f32, storm: f32) -> Self {
        let k = smoothstep(0.15, 0.55, t.star_alpha);
        if k <= 0.0 {
            return Self::build(t, light, cover, storm, false);
        }
        if k >= 1.0 {
            return Self::build(t, light, cover, storm, true);
        }
        let (d, n) = (
            Self::build(t, light, cover, storm, false),
            Self::build(t, light, cover, storm, true),
        );
        let m = |a: Color, b: Color| palette::mix(a, b, k);
        Ink {
            body: m(d.body, n.body),
            top: m(d.top, n.top),
            base: m(d.base, n.base),
            rim: m(d.rim, n.rim),
            rain: m(d.rain, n.rain),
            alpha: d.alpha + (n.alpha - d.alpha) * k,
            lit: d.lit,
            deck: m(d.deck, n.deck),
            deck_base: m(d.deck_base, n.deck_base),
            warm_k: d.warm_k + (n.warm_k - d.warm_k) * k,
            warm_edge: m(d.warm_edge, n.warm_edge),
            warm_base: m(d.warm_base, n.warm_base),
        }
    }

    fn build(t: SkyTint, light: &LightState, cover: f32, storm: f32, night: bool) -> Self {
        let body = palette::mix(
            t.horizon,
            palette::rgb(0xF2, 0xE8, 0xF2),
            if night { 0.10 } else { 0.55 },
        );
        let tinted = palette::rgb(
            (palette::r(body) as f32 * t.ambient[0]).clamp(0.0, 255.0) as u8,
            (palette::g(body) as f32 * t.ambient[1]).clamp(0.0, 255.0) as u8,
            (palette::b(body) as f32 * t.ambient[2]).clamp(0.0, 255.0) as u8,
        );
        let mut body = palette::mix(body, tinted, 0.45);
        // A grey lid: at heavy cover the deck loses its blue and its white.
        if cover > 0.55 {
            let grey = palette::mix(body, palette::rgb(0x8E, 0x93, 0x9C), (cover - 0.55) / 0.45 * 0.55);
            body = grey;
        }
        let moonlit = if light.moon.up(5.0) {
            light.moon_illum.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dusk = if night { 0.0 } else { t.glow.clamp(0.0, 1.0) };
        let warm = palette::rgb(0xFF, 0xB2, 0x7A);
        let mut ink = if night {
            // A dark cloud against a dark sky: only a bright moon picks it out.
            body = palette::mix(body, palette::rgb(0x22, 0x28, 0x38), 0.60);
            let silver = palette::mix(
                palette::rgb(0x38, 0x40, 0x56),
                palette::rgb(0xB4, 0xBE, 0xD4),
                moonlit,
            );
            body = palette::mix(body, silver, 0.45 + 0.35 * moonlit);
            let top = palette::mix(body, palette::rgb(0xC6, 0xD0, 0xE4), 0.16 + 0.34 * moonlit);
            let base = palette::shade(body, 0.74);
            Ink {
                body,
                top,
                base,
                rim: palette::mix(body, palette::rgb(0xA8, 0xB6, 0xD0), 0.55 * moonlit),
                rain: palette::mix(palette::rgb(0x3E, 0x46, 0x58), body, 0.30),
                alpha: 0.78,
                lit: t.sun_x,
                deck: palette::mix(body, palette::rgb(0x2E, 0x34, 0x44), 0.35),
                deck_base: palette::mix(body, palette::rgb(0x14, 0x18, 0x24), 0.55),
                warm_k: 0.0,
                warm_edge: body,
                warm_base: body,
            }
        } else {
            // B90: a day cloud's lit top is near-white (a cumulus is the whitest
            // thing in the sky), warming to the sunset's glow at dusk.
            let top = palette::mix(
                palette::shade(body, 1.10),
                palette::mix(warm, palette::rgb(0xFF, 0xFF, 0xFF), 0.5),
                0.28 * dusk,
            );
            let top = palette::mix(top, palette::rgb(0xFF, 0xFF, 0xFF), 0.42 * (1.0 - dusk));
            let base = palette::mix(
                palette::mix(body, palette::rgb(0x6E, 0x80, 0xA2), 0.58),
                warm,
                0.05 + 0.63 * dusk,
            );
            let rim = palette::mix(
                palette::mix(warm, palette::rgb(0xFF, 0xFF, 0xFF), 0.35),
                body,
                0.35 - 0.25 * dusk,
            );
            Ink {
                body,
                top,
                base,
                rim,
                rain: palette::mix(palette::rgb(0x62, 0x6E, 0x84), body, 0.22),
                alpha: 0.95,
                lit: t.sun_x,
                deck: palette::mix(body, palette::rgb(0x76, 0x7E, 0x8A), 0.35 + 0.25 * cover),
                deck_base: palette::mix(base, palette::rgb(0x4E, 0x56, 0x64), 0.30),
                // The sun's side at dusk: edges and tops glow gold-white, bases
                // go deep orange-red; strongest at the sun, fading away from it.
                warm_k: (dusk * 1.25).clamp(0.0, 1.0),
                warm_edge: palette::mix(warm, palette::rgb(0xFF, 0xF2, 0xD0), 0.55),
                warm_base: palette::mix(warm, palette::rgb(0xD8, 0x50, 0x48), 0.55),
            }
        };
        // A storm is a different colour of sky (B57b item 2): a heavy slate deck,
        // cells whose bases go dark and whose tops still catch the light, and
        // grey rain. The `storm` flag is the weather's own thunder, not a guess
        // from the cover: a 95 % grey day is not a thunderstorm.
        let s = storm.clamp(0.0, 1.0);
        if s > 0.05 {
            let heavy = s * (0.55 + 0.45 * cover);
            // The deck goes to slate and the cells' vaults go darker still, so a
            // cell reads as a *thing* against the lid instead of blending into it.
            ink.deck = palette::mix(ink.deck, palette::rgb(0x3E, 0x44, 0x4E), 0.78 * heavy);
            ink.deck_base = palette::mix(ink.deck_base, palette::rgb(0x22, 0x26, 0x2E), 0.88 * heavy);
            ink.body = palette::mix(ink.body, palette::rgb(0x4A, 0x52, 0x62), 0.85 * heavy);
            ink.base = palette::mix(ink.base, palette::rgb(0x26, 0x2B, 0x35), 0.88 * heavy);
            // The tops keep a *little* light: a cell's cap is lit even when the
            // world under it is not, but only just — a pale cap on a slate deck
            // is the "pale blob" the review saw (B58b item b).
            ink.top = palette::mix(ink.top, palette::rgb(0x76, 0x80, 0x90), 0.62 * heavy);
            // The sun's side of a cell is a *lit slate*, not a white edge: a
            // near-white rim on a dark deck is the halo the review saw.
            ink.rim = palette::mix(ink.rim, palette::rgb(0x9E, 0xA8, 0xB6), 0.42 * heavy);
            ink.rain = palette::mix(ink.rain, palette::rgb(0x46, 0x4E, 0x5E), 0.65 * heavy);
        }
        ink
    }
}

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

/// Tone indices inside a [`Shape`].
const T_BODY: u8 = 0;
const T_TOP: u8 = 1;
const T_BASE: u8 = 2;
const T_RAIN: u8 = 3;

/// One lobe of a cloud: an ellipse, optionally rotated, positioned from the
/// shape's base centre (`cy` upward, px).
#[derive(Clone, Copy)]
struct Lobe {
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    rot: f32,
    /// Force a tone (anvils, rain); `None` shades by height within the lobe.
    tone: Option<u8>,
    /// Peak coverage of this lobe (0..1).
    a: f32,
    /// Clamp the bottom of this lobe to the shape's base (a cloud's flat base).
    flat: bool,
}

/// A baked cloud: coverage (0..255), a tone per pixel, and the blurred coverage a
/// shadow uses — row-major, `w × h`.
#[derive(Clone)]
pub struct Shape {
    pub w: usize,
    pub h: usize,
    cov: Vec<u8>,
    tone: Vec<u8>,
    shade: Vec<u8>,
    /// B90: a sky-baked cloud. Its tone byte is a rung on the seven-step light
    /// ladder (low nibble) and which way the surface faces (bits 4-5), not one
    /// of the four flat tones.
    ladder: bool,
}

impl Shape {
    #[inline]
    fn at(&self, x: usize, y: usize) -> (u8, u8) {
        let k = y * self.w + x;
        (self.cov[k], self.tone[k])
    }
}

/// The tone of a pixel *within its own lobe*: a cumulus reads as a heap of
/// rounded bumps, each lit on top and shaded underneath.
#[inline]
fn tone_in_lobe(v: f32, ry: f32, dither: f32) -> u8 {
    let u = ((v + ry) / (2.0 * ry)).clamp(0.0, 1.0) + dither;
    if u > 0.72 {
        T_BASE
    } else if u < 0.34 {
        T_TOP
    } else {
        T_BODY
    }
}

/// Bake a cloud into a sprite, sized to fit its lobes.
///
/// Fitting is not a detail: B57 sized the canvas from the caller's estimate and
/// a storm's anvil, which stands a little above the rest, was cut by the top of
/// the sprite — a dead straight horizontal edge across a 190-px cloud (the B57b
/// review found it at 4×).
fn bake(lobes: &[Lobe], seed: u64, fringe: bool) -> Shape {
    // The canvas: every lobe's extent, plus the fringe.
    let (mut x0, mut x1, mut y0, mut y1) = (0.0f32, 4.0f32, 0.0f32, 4.0f32);
    for l in lobes {
        // A rotated ellipse reaches `sqrt(rx²cos² + ry²sin²)` sideways and the
        // other way round vertically.
        let (c, s) = (l.rot.cos().abs(), l.rot.sin().abs());
        let ex = (l.rx * l.rx * c * c + l.ry * l.ry * s * s).sqrt();
        let ey = (l.rx * l.rx * s * s + l.ry * l.ry * c * c).sqrt();
        y0 = y0.max(l.cy + ey);
        y1 = y1.max(if l.flat { ey - l.cy } else { 0.0 });
        x0 = x0.max(-l.cx + ex);
        x1 = x1.max(l.cx + ex);
    }
    let w = (x0.max(x1) * 2.0 + 4.0).ceil() as usize;
    let h = (y0 + y1 + 4.0).ceil() as usize;
    let base_y = (h - 1) as f32 - y1.max(0.0);
    let mut cov = vec![0u8; w * h];
    let mut tone = vec![T_BODY; w * h];
    for l in lobes {
        let (lx, ly) = (w as f32 * 0.5 + l.cx, base_y - l.cy);
        let (c, s) = (l.rot.cos(), l.rot.sin());
        let xa = ((lx - l.rx * 1.42 - 2.0) as i32).max(0) as usize;
        let xb = ((lx + l.rx * 1.42 + 3.0) as i32).min(w as i32) as usize;
        let ya = ((ly - l.ry * 1.42 - 2.0) as i32).max(0) as usize;
        let yb = ((ly + l.ry * 1.42 + 3.0) as i32).min(h as i32) as usize;
        for y in ya..yb {
            for x in xa..xb {
                // A cloud's base is flat — but a pixel-art flat, wobbling a
                // pixel every few columns, not a ruled line.
                let wob = if hash((x as i64 / 4) * 13, seed) > 0.5 {
                    1.0
                } else {
                    0.0
                };
                if l.flat && y as f32 > base_y - wob + 0.5 {
                    continue;
                }
                let dx = x as f32 + 0.5 - lx;
                let dy = y as f32 + 0.5 - ly;
                let u = dx * c + dy * s;
                let v = -dx * s + dy * c;
                let d = (u / l.rx).powi(2) + (v / l.ry).powi(2);
                if d > 1.0 {
                    continue;
                }
                // Solid inside. The raggedness is *one* fringe around the union
                // of the lobes (below), never per lobe: a per-lobe ragged edge
                // stipples the seam where two lobes overlap, which reads as a
                // dotted line across the cloud — and, on a dark storm deck, as a
                // halo around every cell (B58b item b).
                let _ = (seed, x, y);
                let k = y * w + x;
                let q = (255.0 * l.a) as u8;
                if q > cov[k] {
                    cov[k] = q;
                    // A dithered seam between the lit top and the shaded base,
                    // so the shading follows each bump instead of banding.
                    let dt = (hash(x as i64 * 5 + y as i64 * 17, seed) - 0.5) * 0.10;
                    tone[k] = l.tone.unwrap_or_else(|| tone_in_lobe(v, l.ry, dt));
                }
            }
        }
    }
    // One ragged, translucent fringe around the *whole* cloud — never inside it:
    // a dither between two overlapping lobes reads as a dotted line across the
    // cloud, which is what the first pass drew.
    //
    // B58b item b: a storm cell gets *no* fringe. Its base merges into the deck,
    // and a stippled pale fringe on a dark deck is exactly the "dithered halo"
    // the review asked to remove.
    if !fringe {
        let shade = blur_mask(&cov, w, h, 3);
        return Shape {
            w,
            h,
            cov,
            tone,
            shade,
            ladder: false,
        };
    }
    let src = cov.clone();
    let solid = |x: usize, y: usize| src[y * w + x] > 0;
    for y in 0..h {
        for x in 0..w {
            if cov[y * w + x] > 0 {
                continue;
            }
            let near = (x > 0 && solid(x - 1, y))
                || (x + 1 < w && solid(x + 1, y))
                || (y > 0 && solid(x, y - 1))
                || (y + 1 < h && solid(x, y + 1));
            if !near {
                continue;
            }
            let k = y * w + x;
            cov[k] = if hash(x as i64 * 3 + y as i64 * 29, seed) > 0.45 {
                120
            } else {
                48
            };
            // The fringe takes the body tone: a bright outline would cut the
            // cloud out of the sky.
            tone[k] = T_BODY;
        }
    }
    let shade = blur_mask(&cov, w, h, 3);
    Shape {
        w,
        h,
        cov,
        tone,
        shade,
        ladder: false,
    }
}

/// Re-tone a whole shape by its height, not by each lobe: a tower built from a
/// stack of lobes would otherwise show a lit top and a dark under-edge *per
/// lobe*, which reads as a stack of pancakes rather than as one cell.
fn tone_by_height(sh: &mut Shape, seed: u64) {
    if sh.h == 0 {
        return;
    }
    for y in 0..sh.h {
        let u = y as f32 / (sh.h.max(2) - 1) as f32;
        for x in 0..sh.w {
            let k = y * sh.w + x;
            if sh.cov[k] == 0 {
                continue;
            }
            // A narrow dither between the three bands: wide jitter drew a dotted
            // row across the cell (B58b item b), so the bands meet in a 1-px
            // zigzag instead.
            let d = (hash(x as i64 * 3 + y as i64 * 41, seed) - 0.5) * 0.03;
            let v = u + d;
            sh.tone[k] = if v < 0.22 {
                T_TOP
            } else if v > 0.58 {
                T_BASE
            } else {
                T_BODY
            };
        }
    }
}

/// Box-blur a coverage mask and rescale it, so a shadow is a soft wash with no
/// straight edge anywhere — a cloud sprite's flat base would otherwise draw a
/// ruled line on the ground.
fn blur_mask(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let r = r.max(1);
    let mut tmp = vec![0u32; w * h];
    let mut out = vec![0u8; w * h];
    // Separable box blur, two passes (a triangle kernel: smoother, still cheap).
    for _ in 0..2 {
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                let xa = x.saturating_sub(r);
                let xb = (x + r).min(w - 1);
                for xx in xa..=xb {
                    sum += src[y * w + xx] as u32;
                    n += 1;
                }
                tmp[y * w + x] = sum / n.max(1);
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                let ya = y.saturating_sub(r);
                let yb = (y + r).min(h - 1);
                for yy in ya..=yb {
                    sum += tmp[yy * w + x];
                    n += 1;
                }
                out[y * w + x] = (sum / n.max(1)) as u8;
            }
        }
        if out.iter().all(|v| *v == 0) {
            break;
        }
    }
    // A blur dims the peak: bring the core back up, so a cloud that covers
    // something still shades it properly.
    let peak = out.iter().copied().max().unwrap_or(0).max(1);
    let k = 255.0 / peak as f32;
    for v in out.iter_mut() {
        *v = ((*v as f32 * k).min(255.0)) as u8;
    }
    out
}

// ---------------------------------------------------------------------------
// B90: sky-baked clouds (cumulus, stratocumulus, cirrus)
// ---------------------------------------------------------------------------

/// Light ladder rungs of a sky-baked cloud, brightest first.
pub const LADDER: usize = 7;

const BAYER4: [[f32; 4]; 4] = [
    [0.0, 8.0, 2.0, 10.0],
    [12.0, 4.0, 14.0, 6.0],
    [3.0, 11.0, 1.0, 9.0],
    [15.0, 7.0, 13.0, 5.0],
];

#[inline]
fn bayer(x: usize, y: usize) -> f32 {
    (BAYER4[y & 3][x & 3] + 0.5) / 16.0
}

/// Fractal value noise, `0..1`, for the baking only.
fn fbm(x: f32, y: f32, seed: u64) -> f32 {
    0.55 * vnoise(x, y, seed)
        + 0.28 * vnoise(x * 2.1 + 17.0, y * 2.1 + 5.0, seed ^ 0x51)
        + 0.17 * vnoise(x * 4.3 + 3.0, y * 4.3 + 29.0, seed ^ 0xA3)
}

/// One bump of a heap: an ellipse standing on the base line.
#[derive(Clone, Copy)]
struct Bump {
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
}

/// Bake a heap of bumps into a lit sprite: domain-warped edges, a soft dithered
/// rim and base, and a tone per pixel from the bump's own surface normal (lit on
/// top, shaded underneath, darkest where the cloud is thick above the base).
/// `holes` (0..1) carves the patchy gaps of a stratocumulus sheet.
fn bake_heap(bumps: &[Bump], w: usize, h: usize, seed: u64, holes: f32, soft_base: f32) -> Shape {
    let mut cov = vec![0u8; w * h];
    let mut tone = vec![0u8; w * h];
    let base_y = h as f32 - 1.0 - soft_base;
    let mid = w as f32 * 0.5;
    // The base line wanders a few px over the width, so it is never ruled.
    let base_wob = |x: f32| (vnoise(x / 47.0, 3.0, seed ^ 0xBA5E) - 0.5) * 5.0;
    // Pass 1: the density field.
    let mut dens = vec![0f32; w * h];
    let mut nxv = vec![0f32; w * h];
    let mut nyv = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Warp the lookup so the outline is cauliflower, not a circle union.
            let wx = (fbm(fx / 11.0, fy / 11.0, seed ^ 0x77) - 0.5) * 9.0
                + (fbm(fx / 4.5, fy / 4.5, seed ^ 0x78) - 0.5) * 3.2;
            let wy = (fbm(fx / 11.0 + 40.0, fy / 11.0, seed ^ 0x79) - 0.5) * 9.0
                + (fbm(fx / 4.5 + 7.0, fy / 4.5, seed ^ 0x7A) - 0.5) * 3.2;
            let (px, py) = (fx + wx - mid, fy + wy);
            let mut best = -1.0f32;
            let (mut bnx, mut bny) = (0.0, 0.0);
            for b in bumps {
                let dx = (px - b.cx) / b.rx;
                let dy = (py - (base_y - b.cy)) / b.ry;
                let d = (dx * dx + dy * dy).sqrt();
                let f = 1.0 - d;
                if f > best {
                    best = f;
                    bnx = dx / d.max(0.2);
                    // Up is negative y: a surface facing up has ny > 0.
                    bny = -dy / d.max(0.2);
                }
            }
            let k = y * w + x;
            dens[k] = best;
            nxv[k] = bnx;
            nyv[k] = bny;
        }
    }
    // Per column: the highest covered row, for "how far under the top" shading.
    let mut top = vec![h as f32; w];
    for x in 0..w {
        for y in 0..h {
            if dens[y * w + x] > 0.10 {
                top[x] = y as f32;
                break;
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            let f = dens[k];
            if f <= 0.0 {
                continue;
            }
            let fy = y as f32 + 0.5;
            let bl = base_y + base_wob(x as f32);
            // The base: a soft dithered fade over the last px, not a ruled line.
            let under = (fy - bl) / (soft_base.max(1.0) + 3.0);
            let base_a = (1.0 - under).clamp(0.0, 1.0);
            if base_a <= 0.0 {
                continue;
            }
            // Patchy holes (stratocumulus): a noise cut-out, softly edged.
            let hole = if holes > 0.0 {
                let n = fbm(x as f32 / 23.0, fy / 9.0, seed ^ 0x4801);
                ((n - holes) / 0.12).clamp(0.0, 1.0)
            } else {
                1.0
            };
            // The rim: coverage ramps over the first ~16 % of the bump radius.
            let rim = (f / 0.16).clamp(0.0, 1.0);
            let a = (rim * base_a * hole).clamp(0.0, 1.0);
            // Four dithered coverage steps.
            let q = a * 3.0 + bayer(x, y);
            let lvl = q.floor().clamp(0.0, 3.0);
            if lvl <= 0.0 {
                continue;
            }
            cov[k] = (lvl / 3.0 * 255.0) as u8;
            // Light: the bump's normal says lit-from-above; the depth under the
            // column's top and the nearness to the base say shade.
            let depth = ((fy - top[x]) / (bl - top[x]).max(8.0)).clamp(0.0, 1.0);
            let ny = nyv[k];
            let s = 0.62 * ny + 0.10 - 1.05 * depth * depth
                + (fbm(x as f32 / 6.0, fy / 6.0, seed ^ 0x91) - 0.5) * 0.30
                + (hash(x as i64 * 7 + y as i64 * 13, seed) - 0.5) * 0.10;
            let rung = if s > 0.52 {
                0
            } else if s > 0.28 {
                1
            } else if s > 0.06 {
                2
            } else if s > -0.22 {
                3
            } else if s > -0.50 {
                4
            } else if s > -0.80 {
                5
            } else {
                6
            };
            let side = if nxv[k] < -0.30 {
                1u8
            } else if nxv[k] > 0.30 {
                2u8
            } else {
                0u8
            };
            tone[k] = rung as u8 | (side << 4);
        }
    }
    let shade = blur_mask(&cov, w, h, 3);
    Shape { w, h, cov, tone, shade, ladder: true }
}

/// A cumulus of `w x h` px: a heap of unequal bumps, taller in the middle,
/// every one different. `style` 0 = humilis (flat, wide), 1 = mediocris,
/// 2 = congestus (a tall tower of bumps).
fn bake_cumulus(w: usize, h: usize, seed: u64, style: u8) -> Shape {
    let (wf, hf) = (w as f32, h as f32);
    let r = |k: i64| hash(k, seed ^ 0xC0DE);
    let n = match style {
        0 => 6 + (r(0) * 3.0) as usize,
        1 => 5 + (r(0) * 3.0) as usize,
        _ => 4 + (r(0) * 3.0) as usize,
    };
    let mut bumps: Vec<Bump> = Vec::new();
    let half = wf * 0.5 - 6.0;
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32;
        // Centre column of the bump, jittered; taller towards the middle but
        // with the peak off-centre so no two clouds share a profile.
        let peak = 0.35 + 0.3 * r(1);
        let cx = (t * 2.0 - 1.0) * half * (0.9 + 0.1 * r(10 + i as i64));
        let env = (1.0 - ((t - peak).abs() * 1.7).powf(1.3)).clamp(0.15, 1.0);
        let jit = 0.78 + 0.32 * r(20 + i as i64);
        let wide = match style {
            0 => 1.5,
            1 => 1.15,
            _ => 0.95,
        };
        // Tallest bump top (cy + ry = 1.62 rad) stays inside the sprite, and no
        // bump reaches past the canvas sides: a warped edge cut by the canvas is
        // a ruled line.
        let rad = (hf * (0.20 + 0.36 * env) * jit)
            .min((hf - 8.0) / 1.62)
            .min((wf * 0.5 - 4.0) / wide)
            .max(4.0);
        let rx = rad * wide;
        let cx = cx.clamp(-(wf * 0.5 - rx - 3.0), wf * 0.5 - rx - 3.0);
        bumps.push(Bump { cx, cy: rad * 0.62, rx, ry: rad });
    }
    // A few small bumps riding the shoulders: the cauliflower detail.
    for i in 0..(2 + n / 2) {
        let j = 40 + i as i64;
        let cx = (r(j) * 2.0 - 1.0) * half * 0.8;
        let rad = hf * (0.10 + 0.12 * r(j + 50));
        // Sits on whatever is highest here.
        let mut top = 0.0f32;
        for b in &bumps {
            let dx = (cx - b.cx) / b.rx;
            if dx.abs() < 1.0 {
                top = top.max(b.cy + b.ry * (1.0 - dx * dx).sqrt());
            }
        }
        let cy = (top - rad * 0.35).min(hf - 6.0 - rad);
        bumps.push(Bump { cx, cy, rx: rad * 1.2, ry: rad });
    }
    bake_heap(&bumps, w, h, seed, 0.0, 3.0)
}

/// A stratocumulus sheet: a long, low field of many small flattened bumps with
/// ragged gaps and a banded, grey underside.
fn bake_strato(w: usize, h: usize, seed: u64) -> Shape {
    let (wf, hf) = (w as f32, h as f32);
    let r = |k: i64| hash(k, seed ^ 0x57A7);
    let n = (w / 15).clamp(8, 30);
    let mut bumps = Vec::new();
    let half = wf * 0.5 - 4.0;
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32;
        let cx = (t * 2.0 - 1.0) * half;
        let env = (1.0 - ((t - 0.5) * 1.8).abs().powf(1.6)).clamp(0.25, 1.0);
        let ry = hf * (0.34 + 0.32 * r(i as i64)) * env;
        bumps.push(Bump {
            cx,
            cy: ry * 0.55,
            rx: ry * (2.2 + 1.4 * r(i as i64 + 90)),
            ry: ry.max(3.5),
        });
    }
    bake_heap(&bumps, w, h, seed, 0.38, 2.0)
}

/// High cirrus: thin, wind-combed streaks, faint, no shading to speak of.
fn bake_cirrus(w: usize, h: usize, seed: u64) -> Shape {
    let mut cov = vec![0u8; w * h];
    let mut tone = vec![0u8; w * h];
    let (wf, hf) = (w as f32, h as f32);
    let tilt = (hash(1, seed) - 0.5) * 0.24;
    for y in 0..h {
        for x in 0..w {
            let u = (x as f32 + 0.5 - wf * 0.5) / (wf * 0.5);
            let yy = y as f32 + 0.5 - hf * 0.5 + u * tilt * wf * 0.5;
            let v = yy / (hf * 0.5);
            // A tapering lens, thicker on one end.
            let lens = 1.0 - (u * u * 0.9 + v * v * (0.9 + 0.5 * u.abs())).sqrt();
            if lens <= 0.0 {
                continue;
            }
            // Streaks: noise stretched far along x, thin in y.
            let n = fbm(x as f32 / 34.0, y as f32 / 2.1, seed ^ 0x3C) * 0.7
                + fbm(x as f32 / 90.0, y as f32 / 5.5, seed ^ 0x3D) * 0.3;
            let a = ((n - 0.40) * 3.2 * lens.sqrt()).clamp(0.0, 1.0);
            let q = a * 3.0 + bayer(x, y);
            let lvl = q.floor().clamp(0.0, 3.0);
            if lvl <= 0.0 {
                continue;
            }
            let k = y * w + x;
            cov[k] = (lvl / 3.0 * 255.0) as u8;
            tone[k] = if v > 0.2 { 3 } else { 1 };
        }
    }
    let shade = blur_mask(&cov, w, h, 3);
    Shape { w, h, cov, tone, shade, ladder: true }
}

/// How a cloud is built, per type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Fair-weather cumulus: flat base, rounded lit top.
    Cumulus,
    /// A stratus sheet: wide, low, flat, low-contrast.
    Stratus,
    /// A towering cell with an anvil and a rain shaft under it (storms only).
    Storm,
    /// High cirrus wisps: thin, faint, tilted.
    Cirrus,
}

fn lobes_for(kind: Kind, w: usize, h: usize, storm_top: bool, alt: f32) -> Vec<Lobe> {
    let (wf, hf) = (w as f32, h as f32);
    // A lobe list as fractions: x from the centre in widths, the *centre* height
    // above the base, and the two radii (a cumulus's in heights — a cloud is
    // wider than it is tall, so a round bump is a small fraction of the width).
    let rows: &[(f32, f32, f32, f32, f32)] = match kind {
        // A heap of rounded bumps on one flat base: the cauliflower outline, with
        // the tallest shoulder off-centre so no two clouds are the same shape.
        Kind::Cumulus => &[
            (-0.21, 0.24, 0.30, 0.28, 1.00),
            (0.22, 0.20, 0.28, 0.24, 1.00),
            (-0.06, 0.36, 0.24, 0.38, 1.00),
            (0.08, 0.40, 0.18, 0.40, 0.98),
            (-0.32, 0.20, 0.15, 0.24, 0.95),
            (0.33, 0.17, 0.13, 0.20, 0.95),
        ],
        // A sheet: long low slabs with an uneven top, low contrast.
        Kind::Stratus => &[
            (0.00, 0.24, 0.48, 0.26, 0.90),
            (-0.26, 0.30, 0.26, 0.22, 0.85),
            (0.27, 0.26, 0.24, 0.20, 0.85),
            (0.03, 0.38, 0.20, 0.22, 0.80),
        ],
        // Cirrus: three thin tilted strokes, the middle one longest.
        Kind::Cirrus => &[
            (-0.17, 0.62, 0.27, 0.12, 0.75),
            (0.00, 0.70, 0.34, 0.11, 0.85),
            (0.18, 0.58, 0.24, 0.10, 0.70),
        ],
        Kind::Storm => &[],
    };
    let mut v: Vec<Lobe> = Vec::new();
    // A cumulus and a cirrus wisp are made of round bumps, whose widths follow
    // the *height*; a stratus sheet is made of long horizontal slabs, so its
    // widths are in widths.
    let rx_of = |rx: f32| match kind {
        Kind::Stratus => (rx * wf).max(2.0),
        _ => (rx * hf).max(2.0),
    };
    for (i, (cx, cy, rx, ry, a)) in rows.iter().enumerate() {
        v.push(Lobe {
            cx: cx * wf,
            cy: cy * hf,
            rx: rx_of(*rx),
            ry: (ry * hf).max(1.6),
            rot: if kind == Kind::Cirrus {
                -0.36 + 0.14 * i as f32
            } else {
                0.0
            },
            tone: None,
            a: *a,
            flat: true,
        });
    }
    if kind == Kind::Storm {
        // A tower with its rain under it. `alt` is the height of the rain shaft;
        // the tower takes the rest, measured up from the skirt.
        let skirt = alt.clamp(0.0, hf * 0.7);
        let t = (hf - skirt).max(hf * 0.5);
        // A cell is a *cumulus grown tall*: the same heap of round bumps, with a
        // couple more stacked on top so it towers, and its tone taken from the
        // sprite's height (lit cap, dark vault). A separate wide cap on a narrow
        // stem reads as a flying saucer at this size, and in-sprite rain streaks
        // read as a picket fence — the rain is the soft shaft drawn under it.
        let base = lobes_for(Kind::Cumulus, w, h, storm_top, 0.0);
        v.extend(base);
        for (i, (cx, cy, rx)) in [
            (-0.10f32, 0.66f32, 0.20f32),
            (0.06, 0.82, 0.17),
            (0.00, 0.94, 0.13),
        ]
        .iter()
        .enumerate()
        {
            v.push(Lobe {
                cx: cx * wf + (hash(i as i64 + 61, 3) - 0.5) * wf * 0.06,
                cy: skirt + t * cy,
                rx: (rx * wf).max(2.0),
                ry: t * 0.15,
                rot: 0.0,
                tone: None,
                a: 0.97,
                flat: false,
            });
            // An anvil: a thin, wide slab at the very top, the way a mature cell
            // spreads out when it hits the tropopause (B58b item b).
            if i == 0 {
                v.push(Lobe {
                    cx: -0.02 * wf,
                    cy: skirt + t * 1.02,
                    rx: (0.30 * wf).max(3.0),
                    ry: (t * 0.075).max(1.6),
                    rot: 0.0,
                    tone: Some(T_TOP),
                    a: 0.92,
                    flat: false,
                });
            }
        }
    }
    v
}

// ---------------------------------------------------------------------------
// The field
// ---------------------------------------------------------------------------

/// Which population a cloud belongs to. Every cloud of every population exists
/// from the moment the field is built; what the cover changes is how much of each
/// one is *there* (its `life`), so a change of weather is a crossfade and never a
/// new sky (B67).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// The invisible shadow field over the land.
    Low,
    /// The cells standing in the deck.
    Cell,
    /// Storm towers (present as the storm builds).
    Tower,
    /// Small clouds high in the band.
    Small,
}

/// How many of each there are at most (the cover decides how many are out).
const N_LOW: usize = 15;
const N_CELL: usize = 36;
const N_TOWER: usize = 36;
const N_SMALL: usize = 25;

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One cloud.
#[derive(Clone, Copy)]
pub struct Cloud {
    pub kind: Kind,
    pub group: Group,
    /// Rank within its group: cloud `i` is out once the cover has `i + 1` of them.
    pub rank: u32,
    /// Centre in the space it lives: tiles for the shadow field, band px for the
    /// sky.
    pub cx: f32,
    pub cy: f32,
    /// How far the sprite's bottom floats above the ground, px (the shadow field).
    pub lift: f32,
    /// Where a storm's rain shaft ends, in band px (0 = no shaft).
    pub shaft: f32,
    /// Index into [`Clouds::shapes`].
    pub shape: usize,
    /// 0..=1 opacity on top of the ink's own.
    pub alpha: f32,
    /// How fast it drifts against the wind's band speed (B90: by height).
    pub speed: f32,
    /// How much of this cloud is out right now, 0..=1: it forms from its dense
    /// core outwards and dissolves back into it (see [`Clouds::life_of`]).
    pub life: f32,
    /// `life` as the last repaint saw it, in 1/32ths (so a repaint is only
    /// queued when the picture can have changed).
    drawn: i16,
    /// Last frame's damage rectangles, for the tick.
    prev: [Rect; 2],
}

/// The height, in px, of the plane a cloud's shadow is projected on (B67). It
/// used to be the terrain under the shadow's centre, so a shadow drifting over a
/// terrace jumped up or down by a whole level (12 px) and its edge rippled as it
/// crossed the land: the "distortion" of a cloud that moves. The shade is a
/// sprite that translates rigidly now, whatever the ground does.
const SHADOW_PLANE: i32 = 0;

/// How far a cloud may hang off the frame before it is wrapped round.
const EDGE_MARGIN: i32 = 220;

/// The deck's underside repeats every this many px (a scrolling texture).
const DECK_LEN: usize = 2048;
/// The deck's texture is a seamless strip [`DECK_LEN`] px long and this tall.
const DECK_TEX_H: usize = 128;
/// B90: the deck repaints one stripe of this many px at a time.
pub const STRIPE_W: usize = 256;

/// The cloud field of one world: an invisible shadow field over the land and the
/// deck and cells of the sky band.
pub struct Clouds {
    /// The shadow field: clouds you never see, whose shadows you do.
    pub lows: Vec<Cloud>,
    pub band: Vec<Cloud>,
    shapes: Vec<Shape>,
    /// What the roster was built for (biome and frame): a rebuild is only worth it
    /// when this moves. The cover never rebuilds it.
    key: i64,
    pub cover: f32,
    pub storm: f32,
    /// Accumulated drift: tiles for the shadow field, px for the band.
    drift: (f32, f32),
    band_drift: f64,
    /// The deck's own drift, px: a deck is higher and further away, so it
    /// crosses the frame more slowly than the cells standing in front of it.
    deck_drift: f64,
    /// Drift velocity, tiles/s and px/s.
    vel: (f32, f32),
    band_vel: f32,
    deck_vel: f32,
    /// The deck's last integer offset, so a step can damage it.
    deck_prev: i32,
    pub map_w: f32,
    pub map_h: f32,
    pub frame_w: f32,
    pub frame_h: f32,
    pub band_h: f32,
    /// Do the clouds throw discrete shadows? 1 on a fair day, fading to 0 as the
    /// cover closes into a deck (whose dimming is the ambient's job).
    pub cast_k: f32,
    /// True while any shadow is thrown.
    pub cast: bool,
    /// 0..=1 how much of a continuous deck the cover earns.
    pub deck: f32,
    /// How grey-dull the cells are (they merge into a deck at heavy cover).
    pub dull_k: f32,
    /// 0..=1 how much of a storm the sky holds.
    pub storm_k: f32,
    /// The `deck` the table below was built for.
    deck_built: f32,
    /// `deck` as the last repaint saw it, in 1/64ths.
    deck_drawn: i16,
    /// The deck, precomputed: its underside's height and its mottling per column
    /// (repeating every [`DECK_LEN`] px). Built when the cover moves, never per
    /// frame — a per-pixel noise cost 20 ms a band repaint.
    deck_u: Vec<f32>,
    /// Darker and lighter bands down the deck, quarter rungs per row.
    deck_layer: Vec<i8>,
    /// What each repaint stripe was last brought up to: its drift (px) and its
    /// thickness (1/64ths). One stripe catches up per tick.
    stripe: Vec<(i32, i16)>,
    stripe_next: usize,
    ticked: bool,
    /// The largest `deck_u`: how low the thickest underside can hang.
    deck_u_max: f32,
    /// The deck's texture: a seamless 128×128 tile of tone offsets, indexed with
    /// the drift. A 1D table made the deck read as vertical stripes, and a
    /// per-pixel noise cost 20 ms a band repaint.
    deck_tex: Vec<i8>,
    seed: u64,
    biome: map::Biome,
}

impl Clouds {
    pub fn new(seed: u64) -> Self {
        Clouds {
            lows: Vec::new(),
            band: Vec::new(),
            shapes: Vec::new(),
            key: i64::MIN,
            cover: 0.0,
            storm: 0.0,
            drift: (0.0, 0.0),
            band_drift: 0.0,
            deck_drift: 0.0,
            vel: (0.0, 0.0),
            band_vel: 0.0,
            deck_vel: 0.0,
            deck_prev: 0,
            map_w: 1.0,
            map_h: 1.0,
            frame_w: 1.0,
            frame_h: 1.0,
            band_h: 1.0,
            cast_k: 1.0,
            cast: true,
            deck: 0.0,
            dull_k: 0.0,
            storm_k: 0.0,
            deck_built: -1.0,
            deck_drawn: 0,
            deck_u: Vec::new(),
            deck_layer: Vec::new(),
            stripe: Vec::new(),
            stripe_next: 0,
            ticked: false,
            deck_u_max: 1.0,
            deck_tex: Vec::new(),
            seed,
            biome: map::Biome::Temperate,
        }
    }

    pub fn set_bounds(&mut self, frame_w: usize, frame_h: usize, band_h: usize, map: &TileMap) {
        self.frame_w = frame_w as f32;
        self.frame_h = frame_h as f32;
        self.band_h = band_h.max(1) as f32;
        self.map_w = map.w as f32;
        self.map_h = map.h as f32;
    }

    pub fn is_empty(&self) -> bool {
        self.lows.is_empty() && self.band.is_empty()
    }

    /// Real cloud cover, in the land's own idiom (D68): a desert's sky is
    /// emptier, a jungle's fuller, a tundra's grey and low. The floor is a couple
    /// of fair-weather cumulus — D72 asks for clouds "all day", never an empty sky.
    pub fn translate_cover(biome: map::Biome, cover: f32) -> f32 {
        let c = cover.clamp(0.0, 1.0);
        match biome {
            map::Biome::Desert => c * 0.62 + 0.04,
            map::Biome::Tundra => (c * 1.05 + 0.10).min(1.0),
            map::Biome::Tropical | map::Biome::Bamboo => (c * 1.12 + 0.10).min(1.0),
            map::Biome::Swamp | map::Biome::Lagoon | map::Biome::Islands => {
                (c * 1.08 + 0.08).min(1.0)
            }
            map::Biome::Highlands | map::Biome::Alpine => (c * 1.06 + 0.06).min(1.0),
            map::Biome::Savanna | map::Biome::Steppe => c * 0.80 + 0.05,
            _ => c,
        }
    }

    /// Follow the weather. The cover and the storm arrive already eased (the
    /// weather book moves them over minutes), and everything derived from them
    /// here is a smooth function — the number of clouds out is a *float* whose
    /// marginal cloud is half-formed — so the sky is never rebuilt and nothing
    /// pops. Returns true when the roster itself was (re)built, which asks for a
    /// full frame; ordinary weather changes are damaged by [`Self::tick`].
    pub fn set(
        &mut self,
        cover: f32,
        storm: f32,
        biome: map::Biome,
        cam: &Camera,
        map: &TileMap,
    ) -> bool {
        let cover = Self::translate_cover(biome, cover);
        self.cover = cover;
        self.storm = storm;
        self.storm_k = smoothstep(0.1, 0.6, storm);
        self.cast_k = 1.0 - smoothstep(0.72, 0.92, cover);
        self.cast = self.cast_k > 0.01;
        self.dull_k = smoothstep(0.74, 0.88, cover);
        // The deck comes on with the cover: a wisp at 55 %, a lid by 90 %.
        self.deck = ((cover - 0.52) / 0.36).clamp(0.0, 1.0);
        let key = biome.id() as i64 * 1_000_003 + self.frame_w as i64 * 31 + self.band_h as i64;
        let rebuilt = key != self.key || self.shapes.is_empty();
        if rebuilt {
            self.key = key;
            self.biome = biome;
            self.build_roster(cam, map);
        }
        if rebuilt || (self.deck > 0.02 && self.deck_u.is_empty()) {
            self.build_deck();
        }
        // B90: the stripes start out at what the sky is (a first frame has no
        // stripe still on its way to it); the bands' tones come with the height.
        let ns = (self.frame_w as usize).div_ceil(STRIPE_W).max(1);
        // Until the first tick every set re-snaps (a still never ticks); after it,
        // the stripes only follow the weather through the tick, which damages them.
        if self.stripe.len() != ns || !self.ticked || rebuilt {
            self.stripe = vec![
                (self.deck_drift.round() as i32, (self.deck * 64.0).round() as i16);
                ns
            ];
            self.stripe_next = 0;
        }
        let rows = self.band_h as usize + 16;
        if self.deck_layer.len() != rows {
            let seed = self.seed ^ 0x1A7E;
            let (p1, p2) = (hash(1, seed) * 6.28, hash(2, seed) * 6.28);
            self.deck_layer = (0..rows)
                .map(|y| {
                    let y = y as f32;
                    ((y / 43.0 + p1).sin() * 4.5 + (y / 17.0 + p2).sin() * 2.5 + (y / 7.0 + p1).sin() * 1.0)
                        .round() as i8
                })
                .collect();
        }
        for i in 0..self.lows.len() {
            self.lows[i].life = self.life_of(&self.lows[i]);
        }
        for i in 0..self.band.len() {
            self.band[i].life = self.life_of(&self.band[i]);
        }
        rebuilt
    }

    /// How much of a cloud is out at the current cover, 0..=1.
    ///
    /// Cloud `rank` of its group is out once the cover holds `rank + 1` of them;
    /// the count is a float, so the marginal cloud forms gradually instead of
    /// appearing. Cirrus thins away as the sky closes, stratus sheets come with
    /// the deck, towers come with the storm.
    pub fn life_of(&self, c: &Cloud) -> f32 {
        let cv = self.cover;
        let rank = c.rank as f32;
        let n = |count: f32| (count - rank).clamp(0.0, 1.0);
        // The cells in the deck: a broken sky is many banks; at a heavy cover the
        // lid is the sky and only a few billows stand out of it.
        let cells = (1.0 + 6.0 * cv + 26.0 * cv * cv) * (1.0 - 0.55 * self.deck) + self.deck;
        match c.group {
            Group::Low => n(2.0 + 12.0 * cv),
            Group::Small => n(2.0 + 22.0 * cv) * (1.0 - self.storm_k),
            Group::Tower => n(cells + 2.0) * self.storm_k,
            Group::Cell => {
                let w = match c.kind {
                    Kind::Cirrus => 1.0 - smoothstep(0.30, 0.55, cv),
                    Kind::Stratus => smoothstep(0.70, 0.85, cv),
                    _ => 1.0,
                };
                n(cells) * w * (1.0 - self.storm_k)
            }
        }
    }

    /// How many clouds are out (a little more than nothing), over the land and in
    /// the band.
    pub fn visible(&self) -> (usize, usize) {
        let f = |v: &Vec<Cloud>| v.iter().filter(|c| c.life > 0.02).count();
        (f(&self.lows), f(&self.band))
    }

    /// The deck's shape, once per few percent of cover: its smooth underside and
    /// the streaky texture across it. Both repeat every [`DECK_LEN`] px so the
    /// drift is a table index — the difference between a 20 ms and a 2 ms repaint.
    ///
    /// B90: the underside is a *smooth* swell (no hanging clumps: those read as
    /// stalactites over the far hills), and the texture is wide and low, long
    /// streaks of a layered stratus, not a 128-px tile that repeats like a grid.
    fn build_deck(&mut self) {
        self.deck_built = self.deck;
        self.deck_u.clear();
        if self.deck <= 0.02 {
            return;
        }
        let seed = self.seed ^ 0xD3C1;
        if self.deck_tex.is_empty() {
            let tseed = self.seed ^ 0x7E37;
            self.deck_tex = vec![0i8; DECK_LEN * DECK_TEX_H];
            for ty in 0..DECK_TEX_H {
                for tx in 0..DECK_LEN {
                    // Long streaks: many cells across, few down, so the grain runs
                    // along the sky the way a layered deck does. Quarter-rung units.
                    let (x, y) = (tx as f32, ty as f32);
                    let n = 0.42 * pn2(x, y, 9, 2, tseed)
                        + 0.30 * pn2(x, y, 23, 4, tseed ^ 0x11)
                        + 0.19 * pn2(x, y, 61, 9, tseed ^ 0x22)
                        + 0.09 * pn2(x, y, 150, 21, tseed ^ 0x33);
                    self.deck_tex[ty * DECK_LEN + tx] =
                        ((n - 0.5) * 34.0).round().clamp(-12.0, 12.0) as i8;
                }
            }
        }
        for k in 0..DECK_LEN {
            let u = k as f32;
            // A slow swell, a mid wave and a small bubbling, all periodic, with a
            // bounded slope (nothing hangs).
            let und = pn1(u, 6, seed) * 0.62 + pn1(u, 17, seed ^ 0x1) * 0.28 + pn1(u, 53, seed ^ 0x2) * 0.10;
            self.deck_u.push(0.66 + 0.32 * und);
        }
        self.deck_u_max = self.deck_u.iter().cloned().fold(0.0f32, f32::max);
    }

    /// Build every cloud of the field once. Sizes are for a middling cover, so a
    /// sprite is the same sprite whatever the sky does; the cover decides how
    /// many are out and how solid (see [`Self::life_of`]).
    fn build_roster(&mut self, cam: &Camera, map: &TileMap) {
        self.lows.clear();
        self.band.clear();
        self.shapes.clear();
        for i in 0..N_LOW {
            self.push_low(i as i64, cam, map);
        }
        for i in 0..N_CELL {
            self.push_band(i as i64, Group::Cell);
        }
        for i in 0..N_TOWER {
            self.push_band(i as i64, Group::Tower);
        }
        // Small clouds high in the band: the sky's own perspective. Even at a
        // heavy cover there are little ones far off, above the big banks.
        for i in 0..N_SMALL {
            self.push_small(i as i64);
        }
        let mut cells = 0;
        for c in self.band.iter_mut() {
            if c.group == Group::Cell {
                c.rank = cells;
                cells += 1;
            }
        }
    }

    /// The cover the sprites are drawn for.
    const SIZE_COVER: f32 = 0.45;

    /// One cloud of the shadow field: invisible, and shaped so its shadow reads.
    fn push_low(&mut self, i: i64, cam: &Camera, map: &TileMap) {
        let seed = self.seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let r = |k: i64| hash(i * 13 + k, seed);
        let c = Self::SIZE_COVER;
        // About two and a half times as wide as it is tall — anything flatter
        // reads as a smear, and these shapes are now only ever seen as shade.
        let w = (110.0 + r(1) * 100.0 + c * 80.0) as usize;
        let h = (w as f32 * (0.38 + 0.10 * r(2))) as usize;
        let alt = 190.0 + r(3) * 110.0;
        let lobes = lobes_for(Kind::Cumulus, w, h, true, 0.0);
        let mut shape = bake(&lobes, seed, true);
        // B90: a cloud shadow on the ground is soft and faint, never a plate: its
        // mask is blurred over a tenth of its width so the edge is a long gradient.
        shape.shade = blur_mask(&shape.shade, shape.w, shape.h, (shape.w / 9).max(6));
        // The blur averages inside the canvas only, so its edge would still be a
        // rectangle: taper the mask to nothing over the outer 18 % on every side.
        let (sw, shh) = (shape.w, shape.h);
        let (mx, my) = ((sw as f32 * 0.18).max(4.0), (shh as f32 * 0.30).max(4.0));
        for y in 0..shh {
            for x in 0..sw {
                let dx = (x.min(sw - 1 - x) as f32 / mx).min(1.0);
                let dy = (y.min(shh - 1 - y) as f32 / my).min(1.0);
                let t = smoothstep(0.0, 1.0, dx) * smoothstep(0.0, 1.0, dy);
                let k = y * sw + x;
                shape.shade[k] = (shape.shade[k] as f32 * t) as u8;
            }
        }
        let shape_idx = self.shapes.len();
        self.shapes.push(shape);
        // Placement: rejection-sample the tile rectangle until the shadow's cloud
        // projects on screen (half the rectangle is off the diamond).
        let mut cx = self.map_w * 0.5;
        let mut cy = self.map_h * 0.5;
        for k in 0..14 {
            let u = hash(i * 31 + k * 3, seed ^ 0xA11);
            let v = hash(i * 31 + k * 3 + 1, seed ^ 0xA11);
            let (tx, ty) = (u * self.map_w, v * self.map_h);
            let z = SHADOW_PLANE;
            let p = cam.project(tx, ty, z);
            let m = (w as f32) * 0.5;
            if p.0 > -m && p.0 < self.frame_w + m && p.1 > self.band_h * 0.5 && p.1 < self.frame_h + 20.0
            {
                cx = tx;
                cy = ty;
                break;
            }
        }
        let _ = map;
        self.lows.push(Cloud {
            kind: Kind::Cumulus,
            group: Group::Low,
            rank: i as u32,
            cx,
            cy,
            lift: alt,
            shaft: 0.0,
            shape: shape_idx,
            speed: 1.0,
            alpha: 1.0,
            life: 0.0,
            drawn: 0,
            prev: [Rect::EMPTY; 2],
        });
    }

    fn push_band(&mut self, i: i64, group: Group) {
        let tower = group == Group::Tower;
        let seed = self.seed ^ 0xBA9D ^ (i as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ if tower { 0x70E } else { 0 };
        let r = |k: i64| hash(i * 11 + k, seed);
        let c = Self::SIZE_COVER;
        // Which kind a cell is is a property of the cloud, not of the weather: the
        // first few are always cumulus (a clear sky is never empty), a few are
        // cirrus wisps that thin away as the sky closes, a few are the stratus
        // sheets that come with a deck.
        let kind = if tower {
            // B58b item b: in a storm every cell is a tower rising out of the
            // deck. Plain pale cumulus in a slate sky is the "pale blobs pasted
            // on the dark deck" the review saw.
            Kind::Storm
        } else if i < 3 {
            Kind::Cumulus
        } else if r(5) < 0.16 {
            Kind::Cirrus
        } else if r(5) > 0.80 {
            Kind::Stratus
        } else {
            Kind::Cumulus
        };
        // B90: three cumulus species (flat humilis, ordinary, tall congestus)
        // and very different sizes, so a sky is never one puff repeated.
        let style: u8 = {
            let q = r(8);
            if q < 0.30 {
                0
            } else if q < 0.78 {
                1
            } else {
                2
            }
        };
        let w = match kind {
            Kind::Cumulus => (match style {
                0 => 150.0 + r(1) * 230.0,
                1 => 110.0 + r(1) * 220.0 + c * 90.0,
                _ => 90.0 + r(1) * 110.0,
            }) as usize,
            Kind::Stratus => (260.0 + r(1) * 340.0) as usize,
            Kind::Cirrus => (170.0 + r(1) * 300.0) as usize,
            Kind::Storm => (110.0 + r(1) * 80.0) as usize,
        };
        let h = match kind {
            Kind::Cumulus => (w as f32
                * match style {
                    0 => 0.20 + 0.08 * r(2),
                    1 => 0.30 + 0.14 * r(2),
                    _ => 0.62 + 0.20 * r(2),
                }) as usize,
            Kind::Stratus => (26.0 + r(2) * 24.0) as usize,
            Kind::Cirrus => (14.0 + r(2) * 14.0) as usize,
            // A cell is a tower, but the band is only so tall: a 110-px tower
            // rising from near the horizon is the most that fits, and a sprite
            // clipped by the top of the frame reads as a flat top.
            Kind::Storm => (72.0 + r(2) * 38.0) as usize,
        };
        // Where it sits in the band, and — since a cloud lower down is further
        // away — how big it is: a big cumulus can only live high up, a small one
        // can sit near the horizon. Without this every cloud is pinned to the top
        // of the band by its own height, leaving the lower half of the sky empty.
        let bands = self.band_h;
        let base_y = if kind == Kind::Storm {
            // A tower must be tall, so its base stays high.
            bands * (0.62 + 0.16 * r(6))
        } else {
            // Most of the sky's clouds are up high; the lower ones are further
            // away and smaller.
            bands * (0.14 + 0.64 * r(4).powf(0.65))
        };
        let max_h = (base_y - 7.0).max(10.0);
        let h_desired = h;
        let h = h.min(max_h as usize).max(10);
        let w = ((w as f32) * (h as f32 / h_desired.max(1) as f32)).max(22.0) as usize;
        let shape = match kind {
            Kind::Storm => {
                let lobes = lobes_for(kind, w, h, true, 0.0);
                let mut shape = bake(&lobes, seed, false);
                tone_by_height(&mut shape, seed);
                shape
            }
            Kind::Cumulus => bake_cumulus(w, h, seed, style),
            Kind::Stratus => bake_strato(w, h, seed),
            Kind::Cirrus => bake_cirrus(w, h, seed),
        };
        let shape_idx = self.shapes.len();
        let sh = shape.h;
        self.shapes.push(shape);
        let span = self.frame_w + 2.0 * EDGE_MARGIN as f32;
        // A storm's base must clear its own tower; the others take the base the
        // size was chosen for.
        // Every cloud's sprite stays below the top of the frame (a cloud cut by
        // the screen edge reads as a flat top; the owner saw it).
        let base_y = (sh as f32 + 3.0).max(base_y);
        self.band.push(Cloud {
            kind,
            group,
            rank: i as u32,
            cx: (r(3) * span).round(),
            cy: base_y.min(bands - 6.0),
            lift: 0.0,
            shaft: if kind == Kind::Storm { bands - 5.0 } else { 0.0 },
            shape: shape_idx,
            // Higher (and nearer overhead) crosses the sky faster; cirrus is far
            // up and slow-looking; the lowest clouds crawl.
            speed: match kind {
                Kind::Cirrus => 0.55 + 0.25 * r(9),
                Kind::Storm => 0.9,
                _ => 0.40 + 0.95 * (1.0 - base_y / bands).clamp(0.0, 1.0) + 0.15 * r(9),
            },
            alpha: match kind {
                Kind::Cumulus => 0.95,
                Kind::Stratus => 0.72 - 0.16 * r(7),
                Kind::Cirrus => 0.34,
                Kind::Storm => 0.92,
            },
            life: 0.0,
            drawn: 0,
            prev: [Rect::EMPTY; 2],
        });
    }

    /// A small cumulus high in the band (B58b item a): the far, high part of a
    /// broken sky, which is what stops the cover reading as a few pasted puffs.
    fn push_small(&mut self, i: i64) {
        let seed = self.seed ^ 0x5A11 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let r = |k: i64| hash(i * 17 + k, seed);
        let c = Self::SIZE_COVER;
        let w = (34.0 + r(1) * 62.0 + c * 30.0) as usize;
        let h = (w as f32 * (0.46 + 0.16 * r(2))) as usize;
        let shape = bake_cumulus(w, h, seed, if r(8) < 0.5 { 0 } else { 1 });
        let sh = shape.h as f32;
        let shape_idx = self.shapes.len();
        self.shapes.push(shape);
        let bands = self.band_h;
        // High up = far away: the top half of the band. The whole sprite stays
        // below the top of the frame (the owner: clouds were cut off at the top).
        let base_y = (bands * (0.05 + 0.42 * r(4).powf(1.2))).max(sh + 3.0);
        let span = self.frame_w + 2.0 * EDGE_MARGIN as f32;
        self.band.push(Cloud {
            kind: Kind::Cumulus,
            group: Group::Small,
            rank: i as u32,
            cx: (r(3) * span).round(),
            cy: base_y.min(bands - 6.0),
            lift: 0.0,
            shaft: 0.0,
            shape: shape_idx,
            speed: 0.55 + 0.9 * (1.0 - base_y / bands).clamp(0.0, 1.0),
            alpha: 0.88,
            life: 0.0,
            drawn: 0,
            prev: [Rect::EMPTY; 2],
        });
    }

    /// The wind: clouds drift the way the real wind blows. `dir_deg` is the
    /// meteorological direction the wind comes *from*, so the drift is `dir+180`.
    pub fn set_wind(&mut self, dir_deg: f32, kmh: f32) {
        // Tiles a second at the world's own scale: a tile is about 20 m and 64 px
        // wide, so an 18 km/h wind (5 m/s) is ~16 px/s.
        let s = 0.018 * kmh.clamp(0.0, 140.0) + 0.030;
        let (tx, ty) = crate::light::toward(dir_deg + 180.0);
        self.vel = (tx * s, ty * s);
        self.band_vel = 22.0 * s;
        // The deck is higher and further off, so it slides by more slowly — and a
        // whole-band repaint is the one expensive thing here, so it steps 2 px at
        // a time rather than every pixel.
        self.deck_vel = self.band_vel * 0.22;
    }

    fn advance(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.5);
        self.drift.0 += self.vel.0 * dt;
        self.drift.1 += self.vel.1 * dt;
        self.band_drift += (self.band_vel * dt) as f64;
        self.deck_drift += (self.deck_vel * dt) as f64;
        if self.map_w > 1.0 && self.map_h > 1.0 {
            self.drift.0 = self.drift.0.rem_euclid(self.map_w);
            self.drift.1 = self.drift.1.rem_euclid(self.map_h);
        }
        let span = self.frame_w + 2.0 * EDGE_MARGIN as f32;
        // B90: clouds drift at their own speeds (`Cloud::speed`), so the shared
        // drift is wrapped only at a distance no cloud's wrap can notice.
        self.band_drift = self.band_drift.rem_euclid(span as f64 * 4096.0);
    }

    /// Tile-space position of a shadow-field cloud right now.
    fn low_xy(&self, c: &Cloud) -> (f32, f32) {
        (
            (c.cx + self.drift.0).rem_euclid(self.map_w.max(1.0)),
            (c.cy + self.drift.1).rem_euclid(self.map_h.max(1.0)),
        )
    }

    /// Screen top-left of a shadow-field cloud.
    fn low_tl(&self, c: &Cloud, cam: &Camera, map: &TileMap) -> (i32, i32) {
        let (tx, ty) = self.low_xy(c);
        let z = SHADOW_PLANE;
        let p = cam.project(tx, ty, z);
        let sh = &self.shapes[c.shape];
        (
            p.0.round() as i32 - sh.w as i32 / 2,
            (p.1 - c.lift).round() as i32 - sh.h as i32,
        )
    }

    /// How far a band cloud has drifted, whole px: higher and nearer clouds
    /// cross the sky faster than the low, far ones (B90).
    #[inline]
    fn drift_of(&self, c: &Cloud) -> f32 {
        (self.band_drift * c.speed as f64).round() as f32
    }

    /// Probe: a band cloud's sprite position before rounding.
    pub fn band_tl_f(&self, c: &Cloud) -> (f32, f32) {
        let span = self.frame_w + 2.0 * EDGE_MARGIN as f32;
        let x = (c.cx + self.drift_of(c)).rem_euclid(span) - EDGE_MARGIN as f32;
        let sh = &self.shapes[c.shape];
        (x - sh.w as f32 / 2.0, c.cy - sh.h as f32)
    }

    /// Probe: a shadow's position before rounding.
    pub fn low_tl_f(&self, c: &Cloud, cam: &Camera, map: &TileMap, light: &LightState) -> (f32, f32) {
        let (tx, ty) = self.low_xy(c);
        let z = SHADOW_PLANE;
        let p = cam.project(tx, ty, z);
        let sh = &self.shapes[c.shape];
        let (ox, oy) = Self::shift(light, c.lift).unwrap_or((0, 0));
        (p.0 - sh.w as f32 / 2.0 + ox as f32, p.1 - c.lift - sh.h as f32 + oy as f32)
    }

    /// Probe: where a band cloud's sprite is drawn.
    pub fn band_tl_pub(&self, c: &Cloud) -> (i32, i32) {
        self.band_tl(c)
    }

    /// Probe: where a shadow-field cloud's *shadow* is drawn.
    pub fn low_tl_pub(&self, c: &Cloud, cam: &Camera, map: &TileMap, light: &LightState) -> (i32, i32) {
        let (sx, sy) = self.low_tl(c, cam, map);
        match Self::shift(light, c.lift) {
            Some((ox, oy)) => (sx + ox, sy + oy),
            None => (sx, sy),
        }
    }

    /// Screen top-left of a band cloud's sprite (its bottom is the tower's base).
    fn band_tl(&self, c: &Cloud) -> (i32, i32) {
        let span = self.frame_w + 2.0 * EDGE_MARGIN as f32;
        // B67: every cloud of the band shares one whole-pixel drift (their starting
        // columns are whole pixels), so they all step on the same frame. Staggered
        // steps repainted a box per cloud per step and overlapping boxes were paid
        // for twice; one step is one repaint of the union.
        let x = (c.cx + self.drift_of(c)).rem_euclid(span) - EDGE_MARGIN as f32;
        let sh = &self.shapes[c.shape];
        (x.round() as i32 - sh.w as i32 / 2, c.cy.round() as i32 - sh.h as i32)
    }

    /// Where a cloud's shadow starts, relative to its sprite: the light's ground
    /// offset plus the height the cloud floats at.
    pub fn shift(light: &LightState, lift: f32) -> Option<(i32, i32)> {
        let c = light.cloud_caster()?;
        let alt = c.alt.max(1.5).to_radians();
        let tiles = lift / (alt.tan() * TILE_W as f32);
        let (dx, dy) = (-c.toward.0 * tiles, -c.toward.1 * tiles);
        Some((
            ((dx - dy) * HALF_W as f32).round() as i32,
            ((dx + dy) * HALF_H as f32).round() as i32 + lift.round() as i32,
        ))
    }

    /// The rectangle a shadow-field cloud's *shadow* occupies. The cloud itself
    /// is never drawn (B57b: the camera is below it), so its sprite is no damage.
    fn low_rects(&self, c: &Cloud, cam: &Camera, map: &TileMap, light: &LightState) -> [Rect; 2] {
        if c.life * self.cast_k <= 0.004 {
            return [Rect::EMPTY; 2];
        }
        let (sx, sy) = self.low_tl(c, cam, map);
        let sh = &self.shapes[c.shape];
        let shadow = match Self::shift(light, c.lift) {
            Some((ox, oy)) => Rect::new(sx + ox, sy + oy, sx + ox + sh.w as i32, sy + oy + sh.h as i32),
            None => Rect::EMPTY,
        };
        [shadow, Rect::EMPTY]
    }

    /// The rectangles a band cloud occupies: its sprite and, for a storm, the
    /// rain shaft below it.
    pub fn band_rects_of(&self, c: &Cloud) -> [Rect; 2] {
        if c.life <= 0.004 {
            return [Rect::EMPTY; 2];
        }
        let (sx, sy) = self.band_tl(c);
        let sh = &self.shapes[c.shape];
        let sprite = Rect::new(sx, sy, sx + sh.w as i32, sy + sh.h as i32);
        let shaft = if c.shaft > 0.0 {
            // B93: `draw_shaft` starts at `0.30 * w` (at least 10 px) either
            // side and fans out to 1.3 times that at its foot; the box
            // used to be a constant 0.16 * w, so a drifting cell left the
            // flanks of its rain behind.
            let half = ((sh.w as f32 * 0.30).max(10.0) * 1.30).ceil() as i32 + 1;
            Rect::new(
                sx + sh.w as i32 / 2 - half,
                c.cy as i32,
                sx + sh.w as i32 / 2 + half,
                c.shaft as i32,
            )
        } else {
            Rect::EMPTY
        };
        [sprite, shaft]
    }

    /// Public for the scene: the shadow field's rectangles (its shadow only).
    pub fn rects_of(&self, c: &Cloud, cam: &Camera, map: &TileMap, light: &LightState) -> [Rect; 2] {
        self.low_rects(c, cam, map, light)
    }

    /// Move everything by `dt`, and append the rectangles that changed. A cloud
    /// only reports when its integer screen position moves, and then only the
    /// **strips** the move exposed or vacated.
    pub fn tick(
        &mut self,
        dt: f32,
        cam: &Camera,
        map: &TileMap,
        light: &LightState,
        out: &mut Vec<Rect>,
    ) -> bool {
        if self.is_empty() {
            return false;
        }
        self.advance(dt);
        let mut moved = false;
        let bounds = Rect::new(0, 0, self.frame_w as i32, self.frame_h as i32);
        for i in 0..self.lows.len() {
            let c = self.lows[i];
            let now = self.low_rects(&c, cam, map, light);
            // The shadow's strength as the last repaint saw it, in 1/32ths: a
            // cloud forming or dissolving repaints when the picture can have moved.
            let q = ((c.life * self.cast_k) * 32.0).round() as i16;
            let fading = q != c.drawn;
            let mut changed = false;
            for k in 0..2 {
                let old = c.prev[k].intersect(&bounds);
                let new = now[k].intersect(&bounds);
                if old != new {
                    changed = true;
                    swept(old, new, out);
                } else if fading && !new.is_empty() {
                    changed = true;
                    out.push(new);
                }
            }
            self.lows[i].drawn = q;
            if changed {
                self.lows[i].prev = now;
                moved = true;
            }
        }
        let band_bounds = Rect::new(0, 0, self.frame_w as i32, self.band_h as i32 + 8);
        for i in 0..self.band.len() {
            let c = self.band[i];
            let now = self.band_rects_of(&c);
            let q = (c.life * 32.0).round() as i16;
            let fading = q != c.drawn;
            let mut changed = false;
            for k in 0..2 {
                let old = c.prev[k].intersect(&band_bounds);
                let new = now[k].intersect(&band_bounds);
                if old != new {
                    changed = true;
                    swept(old, new, out);
                } else if fading && !new.is_empty() {
                    changed = true;
                    out.push(new);
                }
            }
            self.band[i].drawn = q;
            if changed {
                self.band[i].prev = now;
                moved = true;
            }
        }
        // The deck thickening or thinning is a band-wide change too.
        //
        // B90: the deck spans the frame, so a whole-band repaint on every drift
        // step cost 21-24 ms. It is repainted in stripes instead: each stripe
        // keeps the (drift, thickness) it was last drawn with, and *one* stripe
        // is brought up to date per tick, so a step is spread over a few frames
        // and no frame pays for more than one stripe (a 1 px shift between
        // neighbours for a few frames is invisible in a soft deck).
        self.ticked = true;
        let target = (self.deck_drift.round() as i32, (self.deck * 64.0).round() as i16);
        self.deck_prev = target.0;
        self.deck_drawn = target.1;
        let n = self.stripe.len();
        for k in 0..n {
            let i = (self.stripe_next + k) % n;
            let old = self.stripe[i];
            if old == target {
                continue;
            }
            self.stripe[i] = target;
            self.stripe_next = i + 1;
            // Nothing to repaint while the deck is out of the sky on both sides.
            if old.1 > 1 || target.1 > 1 {
                let x0 = (i * STRIPE_W) as i32;
                let x1 = (((i + 1) * STRIPE_W) as i32).min(self.frame_w as i32);
                // Only the rows the deck reaches: its lowest underside at the
                // thicker of the two states, not the whole band.
                let heavy = old.1.max(target.1) as f32 / 64.0;
                let low = (self.band_h * (0.45 + 0.55 * heavy) * self.deck_u_max).ceil() as i32 + 2;
                out.push(Rect::new(x0, 0, x1, low.min(self.band_h as i32)));
                moved = true;
            }
            break;
        }
        moved
    }

    /// Darken the frame with every shadow-field cloud's shadow that touches
    /// `rect` (B57). The mask is the cloud's *blurred* coverage, so the shade has
    /// no straight edge anywhere.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_shadows(
        &self,
        fb: &mut Framebuffer,
        rect: Rect,
        cam: &Camera,
        map: &TileMap,
        light: &LightState,
        lut: &ShadowLut,
        static_layer: &Framebuffer,
    ) {
        if !self.cast || self.lows.is_empty() {
            return;
        }
        // B67: a shadow falls on the ground. The sky band is the band's own
        // clouds (and with the shadows on a fixed plane, one could land across the
        // horizon haze as a dark slab in the sky), so the shade starts at the land.
        let clip = rect.intersect(&fb.clip());
        let clip = Rect::new(clip.x0, clip.y0.max(self.band_h as i32), clip.x1, clip.y1);
        if clip.is_empty() {
            return;
        }
        let w = fb.w;
        for c in &self.lows {
            let strength = c.life * self.cast_k;
            if strength <= 0.004 {
                continue;
            }
            let Some((ox, oy)) = Self::shift(light, c.lift) else {
                return;
            };
            // A shadow forming or dissolving shrinks to its dense core and
            // thins (the same erosion as the cloud's own sprite).
            let thr = (1.0 - strength) * 0.45;
            let scale = strength.min(1.0);
            let (sx, sy) = self.low_tl(c, cam, map);
            let sh = &self.shapes[c.shape];
            let (tx, ty) = (sx + ox, sy + oy);
            let r = Rect::new(tx, ty, tx + sh.w as i32, ty + sh.h as i32).intersect(&clip);
            if r.is_empty() {
                continue;
            }
            for y in r.y0..r.y1 {
                let src_y = (y - ty) as usize;
                let row = y as usize * w;
                for x in r.x0..r.x1 {
                    let mut cov = sh.shade[src_y * sh.w + (x - tx) as usize];
                    if cov == 0 {
                        continue;
                    }
                    if strength < 0.999 {
                        let f = cov as f32 / 255.0;
                        let e = ((f - thr) / (1.0 - thr)).clamp(0.0, 1.0);
                        cov = (e * scale * 255.0) as u8;
                        if cov == 0 {
                            continue;
                        }
                    }
                    let i = row + x as usize;
                    // The sky takes no shadow: there is nothing there to darken.
                    if static_layer.px()[i] == TRANSPARENT {
                        continue;
                    }
                    // B90: faint, not a plate (a cloud's shade is a few tints
                    // darker, and a flat sea shows it worst).
                    // And it fades in below the horizon, not a cut at the land line.
                    let below = ((y - self.band_h as i32) as f32 / 36.0).clamp(0.0, 1.0);
                    let cov = (cov as f32 * 0.59 * below * below * (3.0 - 2.0 * below)) as u8;
                    if cov == 0 {
                        continue;
                    }
                    fb.px_mut()[i] = lut.apply(fb.px()[i], cov);
                }
            }
        }
    }

    /// What fraction of the frame the shadows cover (the sprite areas, clipped to
    /// the frame — the tools report it, nothing depends on it).
    pub fn shadow_area(&self, frame_w: usize, frame_h: usize) -> f64 {
        let mut area = 0.0;
        for c in &self.lows {
            let sh = &self.shapes[c.shape];
            area += (sh.w * sh.h) as f64;
        }
        (area / (frame_w.max(1) * frame_h.max(1)) as f64).min(1.0)
    }

    // -----------------------------------------------------------------------
    // The sky band
    // -----------------------------------------------------------------------

    /// Draw the band: the deck first, then the cells standing in it.
    pub fn draw_band(&self, fb: &mut Framebuffer, ink: &Ink) {
        self.draw_band_over(fb, ink, None);
    }

    /// As [`Self::draw_band`], but never over `solid`'s standing things (B90):
    /// a tower or a landmark that rises into the band is nearer than any cloud,
    /// so a cloud or the deck must not be painted over its roof. `solid` is the
    /// static layer (its alpha byte tags props and buildings).
    pub fn draw_band_over(&self, fb: &mut Framebuffer, ink: &Ink, solid: Option<&[Color]>) {
        let clip = fb.clip();
        if clip.is_empty() {
            return;
        }
        let band = (self.band_h as i32).max(8);
        let top = clip.y0.max(0);
        let bottom = clip.y1.min(band);
        if bottom <= top {
            return;
        }
        self.draw_deck(fb, ink, top, bottom, solid);
        for c in &self.band {
            if c.life <= 0.004 {
                continue;
            }
            let (sx, sy) = self.band_tl(c);
            let sh = &self.shapes[c.shape];
            if c.shaft > 0.0 {
                self.draw_shaft(fb, ink, c, sx + sh.w as i32 / 2, sh);
            }
            let dull = if c.kind == Kind::Storm { 0.0 } else { self.dull_k };
            blit(fb, sh, sx, sy, ink, c.alpha * ink.alpha, band, dull, c.life, solid);
        }
    }

    /// The continuous deck: a lid that fills the band, layered rather than flat.
    ///
    /// B90: the deck is repainted in vertical stripes ([`STRIPE_W`] wide), each
    /// with the drift and thickness it was last *brought up to* (see
    /// [`Self::tick`]), so one drift step costs one stripe a frame, never the
    /// whole 7040 x 480 band. A pixel's tone is a ladder rung picked with an
    /// ordered dither from its depth under the underside, a texture that scrolls
    /// at a different speed on every row (far rows lag), and a few darker and
    /// lighter bands. The underside fades to nothing over ~90 px, so the deck
    /// dissolves into the haze and never ends on a line.
    fn draw_deck(&self, fb: &mut Framebuffer, ink: &Ink, top: i32, bottom: i32, solid: Option<&[Color]>) {
        if self.deck_u.is_empty() || self.stripe.is_empty() {
            return;
        }
        let band = self.band_h;
        let clip = fb.clip();
        let mut tones = [ink.deck; 8];
        let lit = palette::mix(ink.deck, ink.top, 0.55);
        for (i, t) in tones.iter_mut().enumerate() {
            *t = palette::mix(lit, ink.deck_base, i as f32 / 7.0);
        }
        let len = self.deck_u.len() as i32;
        let w = fb.w;
        let rows = (bottom - top).max(0) as usize;
        if rows == 0 {
            return;
        }
        let dmax = band as usize + 16;
        let fade_px = 90.0f32;
        let mut k_of_d = vec![0u32; dmax + 1];
        let mut depth4 = vec![0i32; dmax + 1];
        let mut row_off = vec![0i32; rows];
        let mut base = vec![0i32; STRIPE_W];
        let first = (clip.x0.max(0) as usize) / STRIPE_W;
        let last = ((clip.x1.max(0) as usize).saturating_sub(1) / STRIPE_W).min(self.stripe.len() - 1);
        for si in first..=last {
            let (drift, q) = self.stripe[si];
            let heavy = q as f32 / 64.0;
            if heavy <= 0.02 {
                continue;
            }
            let x0 = (si * STRIPE_W).max(clip.x0.max(0) as usize);
            let x1 = ((si + 1) * STRIPE_W).min(clip.x1.max(0) as usize).min(w);
            if x1 <= x0 {
                continue;
            }
            let reach = band * (0.45 + 0.55 * heavy);
            for d in 0..=dmax {
                let e = (d as f32 / fade_px).min(1.0);
                // Smoothstep: a long soft underside, then solid.
                let a = heavy * 0.97 * e * e * (3.0 - 2.0 * e);
                k_of_d[d] = if a <= 0.02 { 0 } else { (a.clamp(0.0, 1.0) * 255.0) as u32 };
                let f = 1.0 - (d as f32 / (band * 0.85)).min(1.0);
                depth4[d] = (f * f.sqrt() * 18.0) as i32;
            }
            // Far rows lag: the texture's drift falls from 1x overhead to 0.35x
            // at the horizon, so the layers slide over each other.
            for r in 0..rows {
                let y = top + r as i32;
                let m = 1.0 - 0.65 * (y as f32 / band).clamp(0.0, 1.0);
                row_off[r] = (drift as f32 * m).round() as i32;
            }
            for x in x0..x1 {
                let k = (x as i32 + drift).rem_euclid(len) as usize;
                base[x - x0] = (reach * self.deck_u[k]).min(band + 8.0) as i32;
            }
            for r in 0..rows {
                let y = top + r as i32;
                let roff = row_off[r];
                let trow = (y as usize % DECK_TEX_H) * DECK_LEN;
                let layer = self.deck_layer[(y as usize).min(self.deck_layer.len() - 1)] as i32;
                let prow = y as usize * w;
                let bay = &BAYER4[y as usize & 3];
                for x in x0..x1 {
                    let b = base[x - x0];
                    if y >= b {
                        continue;
                    }
                    let d = ((b - y) as usize).min(dmax);
                    let k8 = k_of_d[d];
                    if k8 == 0 {
                        continue;
                    }
                    if let Some(sp) = solid {
                        if is_standing(sp[prow + x]) {
                            continue;
                        }
                    }
                    let mot = self.deck_tex[trow + (x as i32 + roff).rem_euclid(DECK_LEN as i32) as usize] as i32;
                    let v = 4 + depth4[d] + layer + mot + (bay[x & 3] as i32 >> 2);
                    let col = tones[((v >> 2).clamp(0, 7)) as usize];
                    let i = prow + x;
                    let dst = fb.px()[i];
                    let n = 255 - k8;
                    let m1 = |c: u32, s: u32| ((c * n + s * k8) / 255) as u32;
                    fb.px_mut()[i] = palette::with_alpha(
                        palette::rgb(
                            m1(palette::r(dst) as u32, palette::r(col) as u32) as u8,
                            m1(palette::g(dst) as u32, palette::g(col) as u32) as u8,
                            m1(palette::b(dst) as u32, palette::b(col) as u32) as u8,
                        ),
                        palette::alpha(dst),
                    );
                }
            }
        }
    }

    /// A storm cell's rain: a grey ribbon from the tower's base to the horizon,
    /// fanning out as it falls, with streaks down it.
    fn draw_shaft(&self, fb: &mut Framebuffer, ink: &Ink, c: &Cloud, cx: i32, sh: &Shape) {
        let end = (c.shaft as i32).min(self.band_h as i32 - 4);
        let (y0, y1) = (c.cy as i32, end);
        if y1 <= y0 {
            return;
        }
        let span = (sh.w as f32 * 0.30).max(10.0);
        let seed = self.seed ^ 0x5A17;
        let clip = fb.clip();
        let len = (y1 - y0) as f32;
        for y in y0..y1 {
            let t = (y - y0) as f32 / len;
            // It fans out as it falls and thins as it spreads.
            let half = span * (0.55 + 0.75 * t);
            let a0 = 0.46 * (1.0 - 0.45 * t) * (1.0 - t * t * 0.35);
            let xa = (cx as f32 - half) as i32;
            let xb = (cx as f32 + half) as i32;
            for x in xa.max(clip.x0)..xb.min(clip.x1) {
                if x < 0 || x + 1 >= fb.w as i32 {
                    continue;
                }
                let dx = (x as f32 - cx as f32) / half;
                // Soft sides, and a little streak noise so it reads as rain.
                let side = (1.0 - dx * dx).max(0.0);
                let streak = 0.72 + 0.55 * hash(x as i64 * 17 + (y as i64 / 3) * 91, seed);
                let a = a0 * side * streak;
                if a <= 0.02 {
                    continue;
                }
                fb.put_blended(x, y, ink.rain, (a * c.life).min(0.75));
            }
        }
    }
}

/// The damage of a sprite that moved from `old` to `new`: the whole of both
/// boxes, not just the edge it swept.
///
/// B61c reported only the strips the move exposed or vacated. That is right for
/// a flat-coloured box and wrong for a picture: when a cloud's sprite slides by
/// a pixel, *every* pixel inside it takes the value of its neighbour, so the
/// interior has to be repainted too. With strips only, the edges moved and the
/// middle stayed put until something forced a full frame: the cloud stretched
/// and smeared as it drifted, then snapped back (B67: the "distorted clouds").
fn swept(old: Rect, new: Rect, out: &mut Vec<Rect>) {
    match (old.is_empty(), new.is_empty()) {
        (true, true) => {}
        (false, true) => out.push(old),
        (true, false) => out.push(new),
        (false, false) => {
            if old.intersects(&new) {
                out.push(old.union(&new));
            } else {
                out.push(old);
                out.push(new);
            }
        }
    }
}

/// How many cirrus wisps the band carries at this cover (fair weather only).
fn n_band_cirrus(cover: f32) -> usize {
    if cover < 0.45 {
        1 + (cover * 4.0) as usize
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Source-over blit of a shape, with the sun's side lit (B57b item 2: "lit edges
/// by the sun direction") and a fade into the horizon so a cloud never ends on a
/// straight line at the bottom of the band.
///
/// B67: a moving cloud now repaints its whole box on every pixel step, so this
/// is the hot loop of a drifting sky. Everything that does not depend on the
/// pixel's own coverage is hoisted out of it: the four tone colours (with the
/// sun's rim) are worked out once per *column*, the coverage curve is a table,
/// and the row's horizon fade is one number. The pixels are the same.
fn blit(
    fb: &mut Framebuffer,
    sh: &Shape,
    sx: i32,
    sy: i32,
    ink: &Ink,
    alpha: f32,
    band: i32,
    dull_k: f32,
    life: f32,
    solid: Option<&[Color]>,
) {
    let clip = fb.clip();
    let w = fb.w;
    let x0 = sx.max(clip.x0);
    let x1 = (sx + sh.w as i32).min(clip.x1);
    let y0 = sy.max(clip.y0);
    let y1 = (sy + sh.h as i32).min(clip.y1).min(band);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let mid = sx as f32 + sh.w as f32 * 0.5;
    let half = (sh.w as f32 * 0.5).max(1.0);
    // The coverage curve, once (256 entries).
    let curve = cov_curve();
    // The tone colours, per column: [body, top, base, rain].
    let mut base = [ink.body, ink.top, ink.base, ink.rain];
    if dull_k > 0.004 {
        for (t, c) in base.iter_mut().enumerate() {
            let merged = palette::mix(ink.deck, *c, if t as u8 == T_TOP { 0.35 } else { 0.18 });
            *c = palette::mix(*c, merged, dull_k);
        }
    }
    // A cloud forming or dissolving thins and draws in towards its dense core (the
    // blurred coverage, which is smooth), the lobes parting where they were joined
    // thinly: no pixel is re-rasterised, it only stops being shown. The erosion is
    // gentle on purpose: a cloud's core is the *lower* middle of its lobes, whose
    // tone is the shaded base, and a hard erosion left a dark slab behind.
    let erode = life < 0.999;
    let thr = (1.0 - life) * 0.45;
    let scale = life.clamp(0.0, 1.0);
    // B90: the light ladder of a sky-baked cloud, brightest first, with the
    // dull-deck merge applied; a surface that faces the sun steps one rung
    // brighter, one that faces away one rung darker.
    let mut rungs = [0 as Color; LADDER];
    let mut shift_of = [0i32; 3];
    if sh.ladder {
        let top = palette::mix(ink.top, ink.rim, 0.45);
        rungs = [
            top,
            ink.top,
            palette::mix(ink.top, ink.body, 0.5),
            ink.body,
            palette::mix(ink.body, ink.base, 0.5),
            ink.base,
            palette::shade(ink.base, 0.86),
        ];
        if dull_k > 0.004 {
            let lid = palette::mix(ink.deck, ink.top, 0.55);
            for c in rungs.iter_mut() {
                let merged = palette::mix(lid, *c, 0.30);
                *c = palette::mix(*c, merged, dull_k);
            }
        }
        if ink.lit.abs() > 0.12 {
            let toward = if ink.lit > 0.0 { 2 } else { 1 };
            shift_of[toward] = -1;
            shift_of[3 - toward] = 1;
        }
    }
    // B90: how much of the sun's dusk glow reaches each column of this cloud: full
    // on the sun's side, a little everywhere (the whole sky is lit), none for a
    // dark night or a noon sky.
    let glow_on = sh.ladder && ink.warm_k > 0.04;
    let glow_cols: Vec<f32> = if glow_on {
        (x0..x1)
            .map(|x| {
                let side = ((x as f32 - mid) / half) * ink.lit.signum();
                let toward = (0.5 + 0.5 * side.clamp(-1.0, 1.0)).clamp(0.0, 1.0);
                toward
            })
            .collect()
    } else {
        Vec::new()
    };
    let cols: Vec<[Color; 4]> = (x0..x1)
        .map(|x| {
            let side = ((x as f32 - mid) / half) * ink.lit;
            let mut c = base;
            for (t, col) in c.iter_mut().enumerate() {
                let tone = t as u8;
                if side > 0.05 && tone != T_BASE {
                    *col = palette::mix(*col, ink.rim, (side * 0.30).min(0.55));
                } else if side < -0.05 && tone == T_TOP {
                    *col = palette::mix(*col, ink.body, (-side * 0.25).min(0.5));
                }
            }
            c
        })
        .collect();
    for y in y0..y1 {
        let src_y = (y - sy) as usize;
        let row = y as usize * w;
        // Thinning only in the last few px above the land line: a long fade
        // puts a straight, common edge across every cloud at the same height.
        let horizon = ((band - 1 - y) as f32 / 6.0).clamp(0.0, 1.0);
        let ah = alpha * horizon;
        let srow = src_y * sh.w;
        let px = fb.px_mut();
        for x in x0..x1 {
            let k_src = srow + (x - sx) as usize;
            let cov = sh.cov[k_src];
            if cov == 0 {
                continue;
            }
            let mut a = if sh.ladder {
                cov as f32 / 255.0 * ah * (1.0 - 0.5 * dull_k)
            } else {
                curve[cov as usize] * ah
            };
            if erode {
                let f = sh.shade[k_src] as f32 / 255.0;
                a *= ((f - thr) / (1.0 - thr)).clamp(0.0, 1.0) * scale;
            }
            if a <= 0.02 {
                continue;
            }
            if let Some(sp) = solid {
                if is_standing(sp[row + x as usize]) {
                    continue;
                }
            }
            let col = if sh.ladder {
                let t = sh.tone[k_src];
                let rung = ((t & 15) as i32 + shift_of[((t >> 4) & 3) as usize]).clamp(0, LADDER as i32 - 1);
                let mut c = rungs[rung as usize];
                if glow_on {
                    let toward = glow_cols[(x - x0) as usize];
                    let g = ink.warm_k * (0.10 + 0.90 * toward * toward.sqrt());
                    if rung >= 4 {
                        // Bases catch the low sun from underneath: deep orange.
                        c = palette::mix(c, ink.warm_base, g * 0.80);
                    } else {
                        c = palette::mix(c, ink.warm_edge, g * 0.70);
                    }
                    // The thin dithered rim is where the light comes through.
                    if cov < 255 {
                        c = palette::mix(c, ink.warm_edge, g * 0.75);
                    }
                    // Away from the sun the cloud is in the sky's violet shade.
                    c = palette::mix(c, palette::rgb(0x6A, 0x5C, 0x88), ink.warm_k * (1.0 - toward) * 0.32);
                }
                c
            } else {
                cols[(x - x0) as usize][sh.tone[k_src] as usize & 3]
            };
            let i = row + x as usize;
            let dst = px[i];
            let k = (a.clamp(0.0, 1.0) * 255.0) as u32;
            let n = 255 - k;
            let mix1 = |d: u32, s: u32| ((d * n + s * k) / 255) as u32;
            px[i] = palette::with_alpha(
                palette::rgb(
                    mix1(palette::r(dst) as u32, palette::r(col) as u32) as u8,
                    mix1(palette::g(dst) as u32, palette::g(col) as u32) as u8,
                    mix1(palette::b(dst) as u32, palette::b(col) as u32) as u8,
                ),
                palette::alpha(dst),
            );
        }
    }
}

/// Is this static-layer pixel something that stands up (a prop or a building)?
#[inline]
fn is_standing(p: Color) -> bool {
    let a = (p >> 24) as u8;
    a == crate::framebuffer::TAG_PROP || a == crate::framebuffer::TAG_BUILT
}

/// `cov * max(sqrt(cov), 0.35)` for every coverage byte: the soft edge's curve.
fn cov_curve() -> &'static [f32; 256] {
    static CURVE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    CURVE.get_or_init(|| {
        let mut t = [0.0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f32 / 255.0;
            *v = c * c.sqrt().max(0.35);
        }
        t
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::Orb;

    fn field(cover: f32) -> Clouds {
        let vp = map::world_for_viewport(640, 360);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 100;
        let mut c = Clouds::new(9);
        c.set_bounds(640, 360, 100, &map);
        c.set(cover, 0.0, map::Biome::Temperate, &cam, &map);
        c
    }

    fn light(alt: f64, az: f64) -> LightState {
        let mut ls = LightState::default();
        ls.sun = Orb::new(alt, az);
        ls.dir_strength = 1.0;
        ls.ambient = [1.0; 3];
        ls
    }

    /// B93: the rain shaft under a storm cell fans out to about 1.3 times the
    /// width `draw_shaft` starts with; the damage rectangle must reach every
    /// pixel the shaft paints, or a drifting cell leaves its flanks behind.
    #[test]
    fn a_rain_shaft_is_inside_its_damage_rectangle() {
        let vp = map::world_for_viewport(640, 360);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 100;
        let mut f = Clouds::new(9);
        f.set_bounds(640, 360, 100, &map);
        f.set(0.95, 1.0, map::Biome::Temperate, &cam, &map);
        let ink = Ink::new(SkyTint::DAY, &light(40.0, 180.0), 0.95, 1.0);
        let mut seen = 0;
        for i in 0..f.band.len() {
            if f.band[i].shaft <= 0.0 {
                continue;
            }
            f.band[i].life = 1.0;
            let with = {
                let mut fb = Framebuffer::new(640, 360);
                f.draw_band(&mut fb, &ink);
                fb
            };
            let c = f.band[i];
            f.band[i].shaft = 0.0;
            let without = {
                let mut fb = Framebuffer::new(640, 360);
                f.draw_band(&mut fb, &ink);
                fb
            };
            f.band[i] = c;
            let rects = f.band_rects_of(&c);
            for y in 0..360i32 {
                for x in 0..640i32 {
                    let k = y as usize * 640 + x as usize;
                    if with.px()[k] != without.px()[k] {
                        let covered = rects.iter().any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1);
                        assert!(covered, "shaft pixel ({x},{y}) of cloud {i} is outside its damage {rects:?}");
                    }
                }
            }
            seen += 1;
        }
        assert!(seen > 0, "a storm made no rain shaft");
    }

    #[test]
    fn cover_sets_what_the_sky_is() {
        for c in [0.0, 0.2, 0.6, 0.95] {
            let f = field(c);
            assert!(f.visible().1 > 0, "cover {c} drew no clouds at all");
            // The shadow field never goes empty either: a clear day's few
            // cumulus still throw shade somewhere.
            assert!(f.visible().0 > 0, "cover {c} threw no shade");
        }
        // A two-thirds sky has more cells in it than a fifth...
        assert!(
            field(0.6).visible().1 > field(0.2).visible().1,
            "60 % {:?} vs 20 % {:?}",
            field(0.6).visible().1,
            field(0.2).visible().1
        );
        // ...and a deck may have *fewer* cells than either, because the deck is
        // the sky and the cells left are only the billows standing out of it
        // (`the_deck_thickens_with_the_cover` covers that side).
        assert!(field(0.95).visible().1 >= 2);
    }

    #[test]
    fn the_deck_thickens_with_the_cover() {
        assert_eq!(field(0.30).deck, 0.0, "a third of a sky is not a lid");
        let mid = field(0.70).deck;
        let full = field(0.95).deck;
        assert!(mid > 0.2 && mid < 0.9, "70 % deck {mid}");
        assert!(full > 0.95, "95 % deck {full}");
    }

    #[test]
    fn a_deck_casts_no_discrete_shadow() {
        assert!(field(0.30).cast);
        assert!(!field(0.95).cast, "a full deck dims through the ambient, not blobs");
    }

    #[test]
    fn the_same_seed_and_weather_make_the_same_sky() {
        let a = field(0.55);
        let b = field(0.55);
        assert_eq!(a.band.len(), b.band.len());
        for (x, y) in a.band.iter().zip(b.band.iter()) {
            assert_eq!((x.cx, x.cy, x.shape), (y.cx, y.cy, y.shape));
        }
    }

    /// The shadow lies away from the sun, and a lower sun throws it further.
    #[test]
    fn the_shadow_follows_the_light() {
        let noon = Clouds::shift(&light(70.0, 180.0), 200.0).expect("the sun is up");
        let low = Clouds::shift(&light(10.0, 180.0), 200.0).expect("the sun is up");
        assert!(
            noon.1 > 150 && low.1 > 2 * noon.1,
            "noon {noon:?}, low {low:?}"
        );
        assert!(noon.0.abs() <= 2, "a southern sun must not lean sideways: {noon:?}");
        let east = Clouds::shift(&light(30.0, 90.0), 200.0).unwrap();
        assert!(east.0 > 0, "an eastern sun throws the shadow west: {east:?}");
        let mut down = light(-10.0, 180.0);
        down.moon = Orb::DOWN;
        assert!(Clouds::shift(&down, 200.0).is_none());
    }

    /// Night needs a *bright, high* moon for cloud shadows (D72).
    #[test]
    fn a_crescent_or_a_low_moon_casts_no_cloud_shadow() {
        let mut ls = LightState::default();
        ls.sun = Orb::new(-20.0, 180.0);
        ls.moon = Orb::new(40.0, 120.0);
        ls.moon_illum = 0.45;
        ls.dir_strength = 0.2;
        assert!(ls.caster().is_some(), "the cast-shadow rule still sees moonlight");
        assert!(Clouds::shift(&ls, 200.0).is_none(), "half a moon is not enough");
        ls.moon_illum = 0.95;
        assert!(Clouds::shift(&ls, 200.0).is_some(), "a bright moon must cast");
        ls.moon = Orb::new(8.0, 120.0);
        assert!(Clouds::shift(&ls, 200.0).is_none(), "a low moon grazes the ground");
    }

    #[test]
    fn drifting_with_the_wind_moves_the_clouds_and_reports_damage() {
        let vp = map::world_for_viewport(640, 360);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 100;
        let mut c = Clouds::new(3);
        c.set_bounds(640, 360, 100, &map);
        c.set(0.5, 0.0, map::Biome::Temperate, &cam, &map);
        c.set_wind(270.0, 40.0);
        let ls = light(40.0, 200.0);
        let start: Vec<(f32, f32)> = c.lows.iter().map(|x| c.low_xy(x)).collect();
        let mut rects = Vec::new();
        let mut moved = false;
        for _ in 0..120 {
            rects.clear();
            moved |= c.tick(1.0 / 30.0, &cam, &map, &ls, &mut rects);
        }
        assert!(moved, "two seconds of wind moved nothing");
        assert!(!rects.is_empty(), "no damage was reported");
        let any = c
            .lows
            .iter()
            .zip(start.iter())
            .any(|(n, o)| (c.low_xy(n).0 - o.0).abs() > 0.01 || (c.low_xy(n).1 - o.1).abs() > 0.01);
        assert!(any, "the drift did not move a single cloud");
    }

    /// The shadows are on the frame, soft at the edge and with no straight line:
    /// a cloud sprite's flat base must not be drawn on the ground.
    #[test]
    fn shadows_darken_the_frame_with_soft_edges() {
        let vp = map::world_for_viewport(640, 360);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 100;
        let mut c = Clouds::new(4);
        c.set_bounds(640, 360, 100, &map);
        c.set(0.5, 0.0, map::Biome::Temperate, &cam, &map);
        let ls = light(45.0, 180.0);
        let lut = ShadowLut::new([0.58, 0.62, 0.72], 0.9);
        let mut fb = Framebuffer::new(640, 360);
        let mut world = Framebuffer::new(640, 360);
        fb.clear(palette::rgb(0xB0, 0xB0, 0xB0));
        world.clear(palette::rgb(0x40, 0x40, 0x40));
        c.draw_shadows(&mut fb, Rect::new(0, 0, 640, 360), &cam, &map, &ls, &lut, &world);
        let mut tones = [0usize; 8];
        let mut dark = 0;
        for p in fb.px() {
            let v = palette::r(*p) as usize;
            if v < 0xB0 {
                dark += 1;
                tones[(v / 32).min(7)] += 1;
            }
        }
        assert!(dark > 900, "no cloud shadow landed ({dark} px)");
        let bands = tones.iter().filter(|n| **n > 30).count();
        assert!(bands >= 2, "the edge has one tone only: {tones:?}");
    }

    /// B67: a cloud sprite is the same pixels wherever it is. Two draws of the
    /// band on a plain background, a drift apart: every cloud that has no
    /// neighbour to overlap must show the same picture in its new place.
    #[test]
    fn a_cloud_keeps_its_shape_while_it_drifts() {
        for cover in [0.15f32, 0.35, 0.5] {
            let (w, h) = (2400usize, 220usize);
            let vp = map::world_for_viewport(w as i32, 400);
            let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
            let mut cam = Camera::for_viewport(&vp);
            cam.oy += 100;
            let mut f = Clouds::new(9);
            f.set_bounds(w, 400, h, &map);
            f.set(cover, 0.0, map::Biome::Temperate, &cam, &map);
            f.set_wind(250.0, 45.0);
            let ink = Ink::default();
            let bg = palette::rgb(0x40, 0x80, 0xC0);
            let mut a = Framebuffer::new(w, h);
            a.clear(bg);
            f.draw_band(&mut a, &ink);
            let rects_a: Vec<Rect> = f.band.iter().map(|c| f.band_rects_of(c)[0]).collect();
            // 3 seconds of 45 km/h: tens of pixels.
            for _ in 0..90 {
                f.advance(1.0 / 30.0);
            }
            let mut b = Framebuffer::new(w, h);
            b.clear(bg);
            f.draw_band(&mut b, &ink);
            let rects_b: Vec<Rect> = f.band.iter().map(|c| f.band_rects_of(c)[0]).collect();
            let (mut compared, mut differ) = (0usize, 0usize);
            for (i, (ra, rb)) in rects_a.iter().zip(rects_b.iter()).enumerate() {
                let (dx, dy) = (rb.x0 - ra.x0, rb.y0 - ra.y0);
                // Only a cloud that stays inside the frame and overlaps no other
                // one, before or after.
                let inside = |r: &Rect| r.x0 >= 0 && r.x1 <= w as i32 && r.y0 >= 0;
                let alone = rects_a.iter().chain(rects_b.iter()).enumerate().all(|(j, o)| {
                    j % rects_a.len() == i || !o.intersects(ra) && !o.intersects(rb)
                });
                if dy != 0 || dx == 0 || !inside(ra) || !inside(rb) || !alone {
                    continue;
                }
                for y in ra.y0..ra.y1.min(h as i32) {
                    for x in ra.x0..ra.x1 {
                        compared += 1;
                        let pa = a.px()[y as usize * w + x as usize];
                        let pb = b.px()[y as usize * w + (x + dx) as usize];
                        if pa != pb {
                            differ += 1;
                        }
                    }
                }
            }
            println!("cover {cover}: {compared} cloud px compared, {differ} differ");
            assert!(compared > 2000, "cover {cover}: only {compared} px compared");
            assert_eq!(differ, 0, "cover {cover}: {differ} cloud px changed with position");
        }
    }

    /// A dark night must not paint a bright cloud: D72 asks for a silhouette.
    #[test]
    fn ink_is_dark_at_night_and_sunset_coloured_at_dusk() {
        let night = || SkyTint {
            horizon: palette::rgb(39, 52, 90),
            zenith: palette::rgb(12, 18, 42),
            ambient: [0.18, 0.24, 0.62],
            glow: 1.0,
            star_alpha: 1.0,
            sun_x: 0.0,
        };
        let mut ls = LightState::default();
        ls.sun = Orb::new(-30.0, 180.0);
        ls.moon = Orb::DOWN;
        ls.moon_illum = 0.0;
        let dark = Ink::new(night(), &ls, 0.3, 0.0);
        let lum = |c: Color| palette::r(c) as u32 + palette::g(c) as u32 + palette::b(c) as u32;
        assert!(lum(dark.body) < 200, "a moonless cloud must be dark: {:#x}", dark.body);

        let dusk = SkyTint {
            horizon: palette::rgb(255, 158, 72),
            zenith: palette::rgb(196, 106, 134),
            ambient: [1.0, 0.74, 0.52],
            glow: 0.9,
            star_alpha: 0.0,
            sun_x: 0.6,
        };
        let mut sun = LightState::default();
        sun.sun = Orb::new(3.0, 280.0);
        let warm = Ink::new(dusk, &sun, 0.3, 0.0);
        assert!(
            palette::r(warm.base) > palette::b(warm.base) + 30,
            "a sunset cloud's underside must be warm: {:#x}",
            warm.base
        );
        assert!(warm.lit > 0.5, "the sun's side must be known: {}", warm.lit);
    }

    /// B67: no cloud changes colour in one step at dusk: sweep the stars in and
    /// check the ink moves a few levels at a time.
    #[test]
    fn the_ink_crosses_twilight_without_a_step() {
        let ls = LightState::default();
        let tint = |a: f32| SkyTint {
            horizon: palette::rgb(120, 130, 170),
            zenith: palette::rgb(30, 40, 90),
            ambient: [0.6, 0.6, 0.8],
            glow: 0.5,
            star_alpha: a,
            sun_x: 0.3,
        };
        let mut prev = Ink::new(tint(0.0), &ls, 0.4, 0.0);
        let mut worst = 0i32;
        for i in 1..=200 {
            let now = Ink::new(tint(i as f32 / 200.0), &ls, 0.4, 0.0);
            for (a, b) in [(prev.body, now.body), (prev.top, now.top), (prev.base, now.base), (prev.deck, now.deck)] {
                for (x, y) in [(palette::r(a), palette::r(b)), (palette::g(a), palette::g(b)), (palette::b(a), palette::b(b))] {
                    worst = worst.max((x as i32 - y as i32).abs());
                }
            }
            prev = now;
        }
        assert!(worst <= 6, "a cloud colour stepped {worst} levels in one 0.5 % step of dusk");
    }

    /// B57b item 1: the camera is below the clouds, so no cloud sprite is drawn
    /// over the land — only its shadow.
    #[test]
    fn nothing_is_drawn_over_the_land() {
        let vp = map::world_for_viewport(320, 200);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 60;
        let mut c = Clouds::new(5);
        c.set_bounds(320, 200, 60, &map);
        c.set(0.8, 0.0, map::Biome::Temperate, &cam, &map);
        let ls = light(40.0, 200.0);
        let mut fb = Framebuffer::new(320, 200);
        let mut world = Framebuffer::new(320, 200);
        let ink = Ink::default();
        // The ground: everything below the band is "the world".
        world.clear(palette::rgb(0x40, 0x40, 0x40));
        fb.clear(palette::rgb(0x40, 0x40, 0x40));
        let mut before = fb.px().to_vec();
        // A full band draw must not touch a pixel below the band.
        c.draw_band(&mut fb, &ink);
        for y in 60..200 {
            for x in 0..320 {
                let i = y * 320 + x;
                assert_eq!(fb.px()[i], before[i], "a cloud painted over the land at {x},{y}");
            }
        }
        let _ = (&ls, &mut before);
    }

    fn wide_field(cover: f32, w: usize) -> (Clouds, Camera, TileMap) {
        let vp = map::world_for_viewport(w as i32, 1440);
        let map = TileMap::fill(vp.w, vp.h, map::Tile::flat(2), 5);
        let mut cam = Camera::for_viewport(&vp);
        cam.oy += 480;
        let mut c = Clouds::new(7);
        c.set_bounds(w, 1440, 480, &map);
        c.set(cover, 0.0, map::Biome::Temperate, &cam, &map);
        (c, cam, map)
    }

    /// B90: the deck's drift step used to repaint the whole 7040 x 480 band in
    /// one frame (21-24 ms on the desktop). One tick may now hand out at most one
    /// stripe, and a stripe draws in well under the 3 ms ceiling.
    #[test]
    fn a_deck_step_repaints_one_stripe_a_frame() {
        let (mut c, cam, map) = wide_field(0.95, 7040);
        c.set_wind(90.0, 60.0);
        let ls = light(40.0, 200.0);
        let mut out = Vec::new();
        let mut stripes = 0;
        let mut worst_area = 0i64;
        for _ in 0..600 {
            out.clear();
            c.tick(1.0 / 30.0, &cam, &map, &ls, &mut out);
            let deck: Vec<&Rect> = out.iter().filter(|r| r.y0 == 0 && r.y1 >= 380 && r.x0 % STRIPE_W as i32 == 0 && r.x1 - r.x0 <= STRIPE_W as i32).collect();
            assert!(deck.len() <= 1, "{} deck rects in one tick", deck.len());
            for r in deck {
                stripes += 1;
                worst_area = worst_area.max(((r.x1 - r.x0) * (r.y1 - r.y0)) as i64);
            }
        }
        assert!(stripes > 8, "the deck never drifted ({stripes} stripes)");
        assert!(worst_area <= (STRIPE_W * 480) as i64, "a deck rect was {worst_area} px");
    }

    #[test]
    fn a_deck_stripe_draws_in_under_three_ms() {
        let (c, _cam, _map) = wide_field(0.95, 7040);
        let mut fb = Framebuffer::new(7040, 480);
        fb.clear(palette::rgb(0x70, 0x80, 0x90));
        fb.set_clip(Rect::new(0, 0, STRIPE_W as i32, 480));
        let ink = Ink::default();
        c.draw_band(&mut fb, &ink);
        let t = std::time::Instant::now();
        for _ in 0..10 {
            c.draw_band(&mut fb, &ink);
        }
        let ms = t.elapsed().as_secs_f64() * 100.0;
        println!("deck stripe repaint: {ms:.2} ms");
        let bound = if cfg!(debug_assertions) { 40.0 } else { 3.0 };
        assert!(ms < bound, "a stripe took {ms:.2} ms");
    }

    /// B90: the stripes catch up with the drift and thickness, so the picture a
    /// frame shows is never more than a pixel off what a fresh draw would give.
    #[test]
    fn the_stripes_catch_up_with_the_deck() {
        let (mut c, cam, map) = wide_field(0.95, 3000);
        c.set_wind(90.0, 40.0);
        let ls = light(40.0, 200.0);
        let mut out = Vec::new();
        for _ in 0..300 {
            c.tick(1.0 / 30.0, &cam, &map, &ls, &mut out);
        }
        // Let every stripe finish its last step.
        c.set_wind(90.0, 0.0);
        c.deck_vel = 0.0;
        for _ in 0..20 {
            c.tick(1.0 / 30.0, &cam, &map, &ls, &mut out);
        }
        let target = (c.deck_drift.round() as i32, (c.deck * 64.0).round() as i16);
        assert!(c.stripe.iter().all(|s| *s == target), "{:?} vs {target:?}", c.stripe);
    }

    /// B90: clouds of one sky are not one cloud: sizes and shapes differ.
    #[test]
    fn the_band_has_varied_clouds() {
        let c = field(0.5);
        let mut widths: Vec<usize> = c.band.iter().map(|b| c.shapes[b.shape].w).collect();
        widths.sort();
        assert!(widths[widths.len() - 1] > widths[0] * 3, "sizes too alike: {widths:?}");
        let speeds: std::collections::BTreeSet<i32> =
            c.band.iter().map(|b| (b.speed * 20.0) as i32).collect();
        assert!(speeds.len() > 6, "clouds drift at too few speeds");
    }

    /// B90: the deck's underside is a smooth swell: no column is more than two
    /// px from its neighbour, so nothing hangs off it like a stalactite.
    #[test]
    fn the_deck_underside_has_no_spikes() {
        for cover in [0.8, 0.95, 1.0] {
            let (c, _, _) = wide_field(cover, 2000);
            let reach = 480.0 * (0.45 + 0.55 * c.deck);
            let n = c.deck_u.len();
            for k in 0..n {
                let d = (c.deck_u[(k + 1) % n] - c.deck_u[k]).abs() * reach;
                assert!(d < 2.0, "underside jumps {d:.2} px at column {k} (cover {cover})");
            }
        }
    }
}
