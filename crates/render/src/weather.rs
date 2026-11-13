//! The weather's *static* contribution to a frame (B61b): the whole-frame washes
//! (overcast dimming, the fog band, rain mist, snow haze, a lightning flash).
//! The drifting cloud shadows and the falling particles are the *moving* half:
//! they are painted per frame by `weather_fx` (into each head's shared buffer on
//! the Plasma path), because a shadow that moves changes every pixel under it
//! and would therefore dirty the whole frame if it lived in the scene.
//!
//! These used to be painted by `weather_fx::Fx::draw` over the finished frame on
//! every frame, which is why a rainy or overcast desktop was a full-frame
//! composite, a full-frame hash and a full-frame export 30 times a second. They
//! belong to the *scene*: they change only when the weather or the light does, so
//! once composed into the scene they cost nothing until the next weather book
//! tick, and the per-frame work left in `weather_fx` is only the particles that
//! actually move.
//!
//! The maths, colours and fixed-point tables are moved here unchanged from
//! `crates/weather_fx/src/lib.rs` (they were written for the same 1-4-bit-art
//! frame and are exact at coverage 0/1): `weather_fx` builds an [`Ambient`] and
//! draws it, the wallpaper hands the same [`Ambient`] to the scene, and
//! `weather_fx::Fx::draw` applies it with [`Ambient::apply`] so the two paths
//! cannot drift apart (there is a test that compares them pixel for pixel).

use crate::framebuffer::{Framebuffer, Rect};
use crate::palette::{self, Color, rgb};

// ---- overlay colours (moved from weather_fx, unchanged) --------------------
/// Fog wash (pale grey-blue).
pub const FOG: Color = rgb(0xd6, 0xe0, 0xec);
/// Rainground mist (a thin blue-grey haze hugging the lower frame).
pub const MIST: Color = rgb(0xc2, 0xd0, 0xdc);
/// Snow brightening haze (a faint cool lift over the whole frame).
pub const SNOW_HAZE: Color = rgb(0xe6, 0xee, 0xf4);
/// Overcast dim (muted blue-grey).
pub const DIM: Color = rgb(0x60, 0x6c, 0x80);
/// Cloud-shadow blob (the buildings' existing shadow tone).
pub const SHADOW: Color = rgb(0x38, 0x3c, 0x40);
/// Lightning flash (very pale blue-white).
pub const FLASH: Color = rgb(0xf0, 0xf4, 0xff);

/// Per-pixel factors: `out = dst * km / 256 + add` per channel.
pub type Factor = (u32, [u32; 3]);

/// One cloud-shadow ellipse (fixed size, drifting with the wind). Drawn by the
/// overlay (`weather_fx`), not by the scene's ambient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub rx: f32,
    pub ry: f32,
    /// Per-blob darkness multiplier (0..1).
    pub a: f32,
}

impl Blob {
    /// The screen box this blob can touch, clamped to a `w x h` frame.
    pub fn bounds(&self, w: i32, h: i32) -> Rect {
        Rect::new(
            ((self.x - self.rx) as i32).clamp(0, w),
            ((self.y - self.ry) as i32).clamp(0, h),
            ((self.x + self.rx) as i32 + 1).clamp(0, w),
            ((self.y + self.ry) as i32 + 1).clamp(0, h),
        )
    }
}

/// What the weather paints into the *scene* (B61b): the washes. Every field is
/// an intensity the caller derives from `world::Weather` (see
/// `weather_fx::Intensity`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ambient {
    /// Overcast dimming, 0..1.
    pub dim: f32,
    /// Fog, 0..1 (B99/D73: fog hides the FAR land, so it is densest at the land
    /// line and thins towards the viewer; the near city stays clearest).
    pub fog: f32,
    /// Rain mist, 0..1 (only real rain mists the ground).
    pub mist: f32,
    /// Snow haze, 0..1.
    pub haze: f32,
    /// Lightning, 0..1 (dynamic: it changes every frame of a strike).
    pub flash: f32,

    /// Per-row composed factors, one per frame row (B61b). Built by
    /// [`Ambient::rebuild_rows`] for a frame `height`; empty until then.
    rows: Vec<Factor>,
    rows_h: usize,
    /// B99: the land line (the sky band's height in frame rows; 0 = unknown, a
    /// 30 % band is assumed) and the light the fog stands in (the scene's
    /// ambient tint; fog is lit by the same light as everything else, so it is
    /// dark at night instead of a milky wash).
    land: usize,
    air: Option<[f32; 3]>,
}

/// Below this an intensity is not worth a pass over the pixels.
const EPS: f32 = 0.004;

impl Ambient {
    /// A fresh ambient with the five wash intensities set.
    pub fn new(dim: f32, fog: f32, mist: f32, haze: f32, flash: f32) -> Self {
        Self {
            dim,
            fog,
            mist,
            haze,
            flash,
            ..Self::default()
        }
    }

    /// Does this ambient change any pixel at all?
    pub fn is_active(&self) -> bool {
        self.dim * 0.26 > EPS
            || self.flash > EPS
            || self.fog > 0.01
            || (0.20 * self.mist * self.mist).min(0.20) > EPS
            || (0.07 * self.haze).min(0.07) > EPS
    }

    /// Do the washes (the per-row pass) paint anything?
    pub fn washes_active(&self) -> bool {
        self.dim * 0.26 > EPS
            || self.flash > EPS
            || self.fog > 0.01
            || (0.20 * self.mist * self.mist).min(0.20) > EPS
            || (0.07 * self.haze).min(0.07) > EPS
    }

    /// A cheap identity for "did the ambient change since last frame": the
    /// washes at 1/96 (B67: was 1/255; a wash easing in over minutes crossed a
    /// step every second or so, and each step is a full-frame composite — a 96th of
    /// the dimming is a third of a colour level). The scene recomposites once when
    /// this moves.
    pub fn key(&self) -> u32 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 96.0 + 0.5) as u32;
        q(self.dim) | q(self.fog) << 8 | q(self.mist) << 16 | q(self.haze) << 24
    }

    /// The composed factor for one row of a `height`-row frame. Pure and
    /// allocation-free: the scene caches these per row (`rebuild_rows`), and the
    /// tools that draw a one-off frame call it directly.
    pub fn row_factor(&self, y: usize, height: usize) -> Factor {
        let h = height.max(1);
        let consts: [(Color, f32); 3] = [
            (DIM, 0.26 * self.dim),
            (SNOW_HAZE, (0.07 * self.haze).min(0.07)),
            (FLASH, self.flash),
        ];
        let has_const = consts.iter().any(|&(_, a)| a > EPS);
        let fog_a = 0.78 * self.fog;
        let has_fog = self.fog > 0.01;
        let mist_c = (0.20 * self.mist * self.mist).min(0.20);
        if !has_fog && mist_c <= EPS {
            // No band on any row: the cached constants (or nothing).
            return if has_const { compose(&consts) } else { (256, [0, 0, 0]) };
        }
        // Distance from the viewer: 0 at the bottom of the frame, 1 at the land
        // line; above the line the sky keeps thinning with height.
        let land = if self.land > 0 { self.land.min(h - 1) } else { (h * 30) / 100 };
        let far = if y >= land {
            let t = (y - land) as f32 / (h - land).max(1) as f32;
            let k = (1.0 - t.clamp(0.0, 1.0)).powi(2);
            0.18 + 0.82 * k
        } else {
            1.0 - 0.5 * ((land - y) as f32 / land.max(1) as f32).clamp(0.0, 1.0)
        };
        let cov_fog = if has_fog { fog_a * far } else { 0.0 };
        let cov_mist = if mist_c > EPS { mist_c * far } else { 0.0 };
        let lit = |c: Color| match self.air {
            Some(a) => rgb(
                (palette::r(c) as f32 * a[0]).min(255.0) as u8,
                (palette::g(c) as f32 * a[1]).min(255.0) as u8,
                (palette::b(c) as f32 * a[2]).min(255.0) as u8,
            ),
            None => c,
        };
        let mut l = [(FOG, 0.0f32); 5];
        let mut n = 0;
        if cov_fog > 0.003 {
            l[n] = (lit(FOG), cov_fog);
            n += 1;
        }
        if cov_mist > 0.003 {
            l[n] = (lit(MIST), cov_mist);
            n += 1;
        }
        for &(c, a) in &consts {
            if a > EPS {
                l[n] = (c, a);
                n += 1;
            }
        }
        compose(&l[..n])
    }

    /// B99: tell the washes where the land line is and what light the air is in.
    /// Returns true when either changed enough to need a new row table.
    pub fn set_air(&mut self, land: usize, light: [f32; 3]) -> bool {
        let moved = self.land != land
            || self
                .air
                .map_or(true, |a| (0..3).any(|i| (a[i] - light[i]).abs() > 0.02));
        if moved {
            self.land = land;
            self.air = Some(light);
        }
        moved
    }

    /// The number of rows the cached table holds (0 = not built).
    pub fn rows_len(&self) -> usize {
        self.rows.len()
    }

    /// The frame height the row table was last built for (B67). `rows_len` is 0
    /// while the washes are too faint to draw, which made `set_ambient` think the
    /// table was stale and ask for a full frame every frame of a slow fade-in.
    pub fn rows_height(&self) -> usize {
        self.rows_h
    }

    /// Copy the intensities from `a` without touching the cached row table.
    pub fn copy_fields(&mut self, a: &Self) {
        self.dim = a.dim;
        self.fog = a.fog;
        self.mist = a.mist;
        self.haze = a.haze;
        self.flash = a.flash;
    }

    /// Recompute the per-row factors for a frame of `height` rows. This is the
    /// one place the washes are composed into a table; it runs when the weather
    /// changes (a 1 Hz book tick at most), never per frame.
    pub fn rebuild_rows(&mut self, height: usize) {
        self.rows_h = height;
        self.rows.clear();
        if !self.washes_active() {
            return;
        }
        let h = height.max(1);
        self.rows.reserve(h);
        for y in 0..h {
            let f = self.row_factor(y, h);
            self.rows.push(f);
        }
    }

    /// Paint the washes inside `r` (the per-row pass that used to cover the whole
    /// frame every frame).
    fn draw_washes(&self, fb: &mut Framebuffer, r: Rect) {
        let y0 = r.y0.max(0) as usize;
        let y1 = r.y1.min(fb.h as i32).max(0) as usize;
        let x0 = r.x0.max(0) as usize;
        let x1 = r.x1.min(fb.w as i32).max(0) as usize;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        // A built table is the fast path (the scene always has one); without it
        // the row factors are composed on the fly, which allocates nothing.
        let h = fb.h;
        let lut = (self.rows_h == h && self.rows.len() == h).then_some(&self.rows);
        let w = fb.w;
        let px = fb.px_mut();
        for y in y0..y1 {
            let (km, add) = match lut {
                Some(rows) => rows[y],
                None => self.row_factor(y, h),
            };
            if km == 256 && add == [0, 0, 0] {
                continue; // nothing on this row
            }
            let base = y * w;
            for i in base + x0..base + x1 {
                px[i] = mix_pixel(px[i], km, add);
            }
        }
    }

    /// Paint the whole ambient inside `r`: the washes over every pixel of the
    /// rect. (The cloud shadows and the particles are the overlay's per-frame
    /// half — see `weather_fx`.)
    pub fn apply(&self, fb: &mut Framebuffer, r: Rect) {
        if !self.is_active() {
            return;
        }
        let r = r.intersect(&Rect::from_size(fb.w, fb.h));
        if r.is_empty() {
            return;
        }
        self.draw_washes(fb, r);
    }

}

/// Compose a list of (colour, coverage) washes into one `km/add` pair.
/// Sequential "source-over" with 8-bit fixed point: each stage
/// `out = dst·km/256 + add` feeds the next, so `n` washes become one pass.
pub fn compose(list: &[(Color, f32)]) -> Factor {
    let (mut km, mut add) = (256u32, [0u32; 3]);
    for &(c, a) in list {
        if a <= 0.0 {
            continue;
        }
        let (k2, a2) = fast_blend_table(c, a);
        km = (km * k2) >> 8;
        for ch in 0..3 {
            add[ch] = ((add[ch] * k2 + 128) >> 8) + a2[ch];
        }
    }
    (km, add)
}

// ---- integer fast blend -----------------------------------------------------
//
// `mix_pixel` replaces the f32 `palette::blend` in the full-frame washes
// (dim / fog / flash): out = dst·k + add, per channel, 8-bit fixed point.
// Exact at coverage 0/1, within ±1/256 of the f32 version everywhere else —
// invisible on 10-50 % washes over a 1-4-bit-art-class frame.

/// The per-pixel factors for blending colour `c` over `dst` with coverage `a`.
#[inline]
pub fn fast_blend_table(c: Color, a: f32) -> Factor {
    let a = a.clamp(0.0, 1.0);
    let am = (a * 256.0 + 0.5) as u32; // alpha, fixed point
    let km = 256 - am;
    let add = [
        (palette::r(c) as u32 * am + 128) / 256,
        (palette::g(c) as u32 * am + 128) / 256,
        (palette::b(c) as u32 * am + 128) / 256,
    ];
    (km, add)
}

#[inline]
pub fn mix_pixel(dst: Color, km: u32, add: [u32; 3]) -> Color {
    let r = (((dst >> 16) & 0xFF) * km >> 8) + add[0];
    let g = (((dst >> 8) & 0xFF) * km >> 8) + add[1];
    let b = ((dst & 0xFF) * km >> 8) + add[2];
    0xFF00_0000 | ((r.min(255) as u32) << 16) | ((g.min(255) as u32) << 8) | b.min(255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize) -> Framebuffer {
        let mut fb = Framebuffer::new(w, h);
        for (i, p) in fb.px_mut().iter_mut().enumerate() {
            *p = 0xFF00_0000 | ((i * 7) as u32 & 0xFF) << 16 | ((i * 13) as u32 & 0xFF);
        }
        fb
    }

    /// The same ambient applied to a rect must equal the ambient applied to the
    /// whole frame, inside that rect, and must not touch a pixel outside it.
    #[test]
    fn applying_a_rect_equals_applying_the_frame_inside_it() {
        let a = Ambient {
            dim: 0.7,
            fog: 0.5,
            mist: 0.4,
            haze: 0.3,
            flash: 0.05,
            ..Ambient::default()
        };
        let (w, h) = (200usize, 120usize);
        let mut whole = frame(w, h);
        let mut partial = frame(w, h);
        let mut built = a.clone();
        built.rebuild_rows(h);
        built.apply(&mut whole, Rect::from_size(w, h));
        let r = Rect::new(30, 20, 150, 100);
        built.apply(&mut partial, r);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if x >= 30 && x < 150 && y >= 20 && y < 100 {
                    assert_eq!(partial.px()[i], whole.px()[i], "inside ({x},{y})");
                } else {
                    assert_eq!(partial.px()[i], frame(w, h).px()[i], "outside ({x},{y}) changed");
                }
            }
        }
    }

    /// Two rects that tile the frame add up to the whole frame, which is what a
    /// damage-limited composite relies on.
    #[test]
    fn two_tiles_of_a_rect_make_the_rect() {
        let a = Ambient {
            dim: 0.6,
            fog: 0.8,
            mist: 0.0,
            haze: 0.0,
            flash: 0.0,
            ..Ambient::default()
        };
        let (w, h) = (160usize, 120usize);
        let mut built = a.clone();
        built.rebuild_rows(h);
        let mut whole = frame(w, h);
        built.apply(&mut whole, Rect::from_size(w, h));
        let mut split = frame(w, h);
        built.apply(&mut split, Rect::new(0, 0, w as i32, 60));
        built.apply(&mut split, Rect::new(0, 60, w as i32, h as i32));
        assert_eq!(split.px(), whole.px());
    }

    /// A clear day paints nothing at all.
    #[test]
    fn a_clear_ambient_paints_nothing() {
        let a = Ambient::default();
        assert!(!a.is_active());
        let mut fb = frame(40, 30);
        let before = fb.px().to_vec();
        a.apply(&mut fb, Rect::from_size(40, 30));
        assert_eq!(before, fb.px().to_vec());
    }

}
