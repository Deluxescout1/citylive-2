//! Procedural iso boxes with windows: the static city fabric.

use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::{Camera, Pt};
use crate::palette::{self, Color, pal};
use map::rng::Rng;

/// One window pane on a wall, in wall space (`u` along the edge, `v` from roof
/// edge down to the pavement), normalised to the wall's own width/height.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub wall: u8,
    pub u0: f32,
    pub u1: f32,
    pub v0: f32,
    pub v1: f32,
    pub color: Color,
}

#[derive(Clone, Debug)]
pub struct Building {
    pub tx: i32,
    pub ty: i32,
    pub fw: i32,
    pub fh: i32,
    /// Ground level in pixels.
    pub base_z: i32,
    /// Wall height in pixels above `base_z`.
    pub height: i32,
    pub wall: Color,
    pub roof: Color,
    pub windows: Vec<Window>,
    pub door: bool,
    /// Screen bounds, cached: culling and dirty-region tests use this.
    pub bounds: Rect,
}

const WIN_W: f32 = 3.0;
const WIN_H: f32 = 4.0;
const WIN_PITCH_X: f32 = 6.0;
const WIN_PITCH_Y: f32 = 7.0;

impl Building {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cam: &Camera,
        tx: i32,
        ty: i32,
        fw: i32,
        fh: i32,
        base_z: i32,
        height: i32,
        wall: Color,
        roof: Color,
        rng: &mut Rng,
    ) -> Self {
        let mut b = Self {
            tx,
            ty,
            fw,
            fh,
            base_z,
            height,
            wall,
            roof,
            windows: Vec::new(),
            door: rng.chance(0.35),
            bounds: Rect::EMPTY,
        };
        b.bounds = b.compute_bounds(cam);
        b.fill_windows(rng);
        b
    }

    #[inline]
    fn z_top(&self) -> i32 {
        self.base_z + self.height
    }

    #[inline]
    fn corners(&self) -> (f32, f32, f32, f32) {
        (
            self.tx as f32,
            self.ty as f32,
            (self.tx + self.fw) as f32,
            (self.ty + self.fh) as f32,
        )
    }

    /// Top edge start, top edge end, bottom edge start, bottom edge end for a wall.
    /// `wall` 0 is the right face (`x = tx+fw`), 1 the left face (`y = ty+fh`).
    fn wall_corners(&self, cam: &Camera, wall: u8) -> (Pt, Pt, Pt, Pt) {
        let (x0, y0, x1, y1) = self.corners();
        let (zt, zb) = (self.z_top(), self.base_z);
        if wall == 0 {
            (
                cam.project(x1, y0, zt),
                cam.project(x1, y1, zt),
                cam.project(x1, y0, zb),
                cam.project(x1, y1, zb),
            )
        } else {
            (
                cam.project(x0, y1, zt),
                cam.project(x1, y1, zt),
                cam.project(x0, y1, zb),
                cam.project(x1, y1, zb),
            )
        }
    }

    fn compute_bounds(&self, cam: &Camera) -> Rect {
        let (x0, y0, x1, y1) = self.corners();
        let (zt, zb) = (self.z_top(), self.base_z);
        let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            for z in [zt, zb] {
                let p = cam.project(x, y, z);
                r.x0 = r.x0.min(p.0.floor() as i32);
                r.y0 = r.y0.min(p.1.floor() as i32);
                r.x1 = r.x1.max(p.0.ceil() as i32);
                r.y1 = r.y1.max(p.1.ceil() as i32);
            }
        }
        // A pixel of slack for the roof rim and window rounding.
        r.expanded(1)
    }

    /// Wall edge length in screen pixels (walls are the same length on both
    /// visible faces only when the footprint is square).
    fn wall_len_px(&self, wall: u8) -> f32 {
        let tiles = if wall == 0 { self.fh } else { self.fw } as f32;
        tiles * (16.0f32 * 16.0 + 8.0 * 8.0).sqrt()
    }

    fn fill_windows(&mut self, rng: &mut Rng) {
        let h = self.height as f32;
        for wall in 0..2u8 {
            let len = self.wall_len_px(wall);
            let cols = (((len - 2.0) / WIN_PITCH_X).floor().max(1.0)) as i32;
            let rows = (((h - 3.0) / WIN_PITCH_Y).floor().max(1.0)) as i32;
            if cols <= 0 || rows <= 0 || h < 8.0 {
                continue;
            }
            let total_w = cols as f32 * WIN_PITCH_X - (WIN_PITCH_X - WIN_W);
            let pad_u = ((len - total_w) * 0.5).max(0.0);
            for j in 0..rows {
                for i in 0..cols {
                    let u_px = pad_u + i as f32 * WIN_PITCH_X;
                    let v_px = 3.0 + j as f32 * WIN_PITCH_Y;
                    let roll = rng.f32();
                    let color = if roll < 0.36 {
                        pal::WINDOW_LIT
                    } else if roll < 0.42 {
                        pal::WINDOW_LIT_COOL
                    } else {
                        pal::WINDOW_DARK
                    };
                    self.windows.push(Window {
                        wall,
                        u0: u_px / len,
                        u1: (u_px + WIN_W) / len,
                        v0: v_px / h,
                        v1: (v_px + WIN_H) / h,
                        color,
                    });
                }
            }
        }
    }

    fn wall_point(&self, cam: &Camera, wall: u8, u: f32, v: f32) -> Pt {
        let (ta, tb, ba, bb) = self.wall_corners(cam, wall);
        let tx = ta.0 + (tb.0 - ta.0) * u;
        let ty = ta.1 + (tb.1 - ta.1) * u;
        let bx = ba.0 + (bb.0 - ba.0) * u;
        let by = ba.1 + (bb.1 - ba.1) * u;
        (tx + (bx - tx) * v, ty + (by - ty) * v)
    }

    pub fn draw(&self, fb: &mut Framebuffer, cam: &Camera) {
        self.draw_with_snow(fb, cam, 0.0)
    }

    /// B76 r3: the same draw with snow lying on the roof. The B26 massing's roof
    /// is a flat diamond at the wall top, so the load lies on that plane — the
    /// same rule the kit's decks follow — and the rim keeps a white lip.
    pub fn draw_with_snow(&self, fb: &mut Framebuffer, cam: &Camera, snow: f32) {
        let (x0, y0, x1, y1) = self.corners();
        let (zt, zb) = (self.z_top(), self.base_z);

        // Contact shadow: the footprint diamond one pixel lower than the walls.
        // This covers the 1-px sliver of ground that used to peek out at the
        // front corner (H0 known problem 2) and grounds the building.
        let shadow = [
            cam.project(x0, y0, zb - 1),
            cam.project(x1, y0, zb - 1),
            cam.project(x1, y1, zb - 1),
            cam.project(x0, y1, zb - 1),
        ];
        fb.fill_poly(&shadow, pal::BUILDING_SHADOW);

        // Faces: light from the upper left, so the left face is lighter.
        let right = [
            cam.project(x1, y0, zt),
            cam.project(x1, y1, zt),
            cam.project(x1, y1, zb),
            cam.project(x1, y0, zb),
        ];
        let left = [
            cam.project(x0, y1, zt),
            cam.project(x1, y1, zt),
            cam.project(x1, y1, zb),
            cam.project(x0, y1, zb),
        ];
        fb.fill_poly(&left, palette::shade(self.wall, 0.90));
        fb.fill_poly(&right, palette::shade(self.wall, 0.68));

        // Dark plinth so buildings sit on the pavement.
        let band = (self.height / 12).clamp(2, 4) as f32;
        let v0 = 1.0 - band / self.height as f32;
        for wall in 0..2u8 {
            let q = [
                self.wall_point(cam, wall, 0.0, v0),
                self.wall_point(cam, wall, 1.0, v0),
                self.wall_point(cam, wall, 1.0, 1.0),
                self.wall_point(cam, wall, 0.0, 1.0),
            ];
            fb.fill_poly(&q, palette::shade(self.wall, 0.45));
        }

        for w in &self.windows {
            let q = [
                self.wall_point(cam, w.wall, w.u0, w.v0),
                self.wall_point(cam, w.wall, w.u1, w.v0),
                self.wall_point(cam, w.wall, w.u1, w.v1),
                self.wall_point(cam, w.wall, w.u0, w.v1),
            ];
            let shade_f = if w.wall == 0 { 0.88 } else { 1.0 };
            fb.fill_poly(&q, palette::shade(w.color, shade_f));
        }

        // Entrance on the left face, if this building has one.
        if self.door && self.height >= 12 {
            let len = self.wall_len_px(1);
            let dw = 4.0 / len;
            let du = 0.5 - dw * 0.5;
            let dv = 8.0 / self.height as f32;
            let q = [
                self.wall_point(cam, 1, du, 1.0 - dv),
                self.wall_point(cam, 1, du + dw, 1.0 - dv),
                self.wall_point(cam, 1, du + dw, 1.0),
                self.wall_point(cam, 1, du, 1.0),
            ];
            fb.fill_poly(&q, pal::DOOR);
        }

        // Roof: the footprint diamond at the top, plus a darker rim on the
        // two near edges so the silhouette reads.
        let roof_q = [
            cam.project(x0, y0, zt),
            cam.project(x1, y0, zt),
            cam.project(x1, y1, zt),
            cam.project(x0, y1, zt),
        ];
        let roof_face = if snow > 0.02 {
            crate::palette::mix(self.roof, crate::palette::rgb(0xf4, 0xf8, 0xff), (0.95 * snow).min(0.96))
        } else {
            self.roof
        };
        fb.fill_poly(&roof_q, roof_face);

        let rim_v = 2.0 / self.height as f32;
        for wall in 0..2u8 {
            let q = [
                self.wall_point(cam, wall, 0.0, 0.0),
                self.wall_point(cam, wall, 1.0, 0.0),
                self.wall_point(cam, wall, 1.0, rim_v),
                self.wall_point(cam, wall, 0.0, rim_v),
            ];
            // The rim is the coping: under snow it is the lip the load hangs over.
            let rim = if snow > 0.15 {
                crate::palette::mix(
                    palette::shade(self.roof, 0.80),
                    crate::palette::rgb(0xf0, 0xf6, 0xfd),
                    (0.25 + 0.6 * snow).min(0.9),
                )
            } else {
                palette::shade(self.roof, 0.80)
            };
            fb.fill_poly(&q, rim);
        }
    }
}
