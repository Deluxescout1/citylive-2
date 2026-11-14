//! Isometric projection and quadrilateral/polygon rasterizers with depth.

use render::palette::Color;
use crate::types::Sprite;

pub const TW: i32 = 64;
pub const HW: i32 = 32;
pub const HH: i32 = 16;

thread_local! {
    /// B100: the plan scale a building is being fitted to its lot with: about
    /// `(cx, cy)`, every tile-space `(x, y)` is pulled towards the centre by
    /// `k` (heights are untouched). `1.0` outside a fit.
    static FIT: std::cell::Cell<(f32, f32, f32)> = const { std::cell::Cell::new((0.0, 0.0, 1.0)) };
}

/// Draw with every plan coordinate scaled by `k` about the footprint centre
/// `(cx, cy)` (B100's fit-to-lot).
pub fn with_fit<R>(cx: f32, cy: f32, k: f32, f: impl FnOnce() -> R) -> R {
    let old = FIT.with(|c| c.replace((cx, cy, k)));
    let r = f();
    FIT.with(|c| c.set(old));
    r
}

/// Project 3D tile-space coordinates (x, y in tiles, z in pixels) to 2D screen space.
#[inline]
pub fn project(x: f32, y: f32, z: f32, ox: f32, oy: f32) -> (f32, f32) {
    let (cx, cy, k) = FIT.with(|c| c.get());
    let (x, y) = if k == 1.0 { (x, y) } else { (cx + (x - cx) * k, cy + (y - cy) * k) };
    (
        ox + (x - y) * (HW as f32),
        oy + (x + y) * (HH as f32) - z,
    )
}

/// Project integer tile coordinates to screen.
#[inline]
pub fn project_i(x: i32, y: i32, z: i32, ox: i32, oy: i32) -> (i32, i32) {
    (
        ox + (x - y) * HW,
        oy + (x + y) * HH - z,
    )
}

/// A quadrilateral in screen space with bilinear parameterization.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub tl: (f32, f32),
    pub tr: (f32, f32),
    pub br: (f32, f32),
    pub bl: (f32, f32),
    pub z_base: i32,
    pub z_top: i32,
}

impl Quad {
    pub fn new(tl: (f32, f32), tr: (f32, f32), br: (f32, f32), bl: (f32, f32), z_base: i32, z_top: i32) -> Self {
        Self { tl, tr, br, bl, z_base, z_top }
    }

    /// Approximate height in rows.
    pub fn rows(&self) -> i32 {
        let dl = (self.bl.1 - self.tl.1).hypot(self.bl.0 - self.tl.0);
        let dr = (self.br.1 - self.tr.1).hypot(self.br.0 - self.tr.0);
        dl.max(dr).round() as i32 + 1
    }

    /// Point at (u, v) along quad, where u in 0..=1 and v in 0..=rows.
    #[inline]
    pub fn pt(&self, u: f32, v: f32, rows: f32) -> (f32, f32) {
        let t = if rows > 0.0 { v / rows } else { 0.0 };
        let lx = self.tl.0 + (self.bl.0 - self.tl.0) * t;
        let ly = self.tl.1 + (self.bl.1 - self.tl.1) * t;
        let rx = self.tr.0 + (self.br.0 - self.tr.0) * t;
        let ry = self.tr.1 + (self.br.1 - self.tr.1) * t;
        (lx + (rx - lx) * u, ly + (ry - ly) * u)
    }

    pub fn top_len(&self) -> i32 {
        (self.tr.0 - self.tl.0).hypot(self.tr.1 - self.tl.1).round() as i32
    }

    /// Sub-quad addressed in local face pixels: u0px..u1px, v0px..v1px.
    pub fn sub_px(&self, u0px: i32, v0px: i32, u1px: i32, v1px: i32, rows: f32) -> Quad {
        let w = self.top_len().max(1) as f32;
        let u0 = u0px as f32 / w;
        let u1 = u1px as f32 / w;
        let z_range = (self.z_top - self.z_base) as f32;
        let sub_z_top = self.z_top - (v0px as f32 / rows * z_range) as i32;
        let sub_z_base = self.z_top - (v1px as f32 / rows * z_range) as i32;
        Quad::new(
            self.pt(u0, v0px as f32, rows),
            self.pt(u1, v0px as f32, rows),
            self.pt(u1, v1px as f32, rows),
            self.pt(u0, v1px as f32, rows),
            sub_z_base,
            sub_z_top,
        )
    }

    /// Paint with procedural shader callback: `f(u_pixel, v_pixel, screen_x, screen_y) -> (Color, depth)`.
    pub fn paint(
        &self,
        sprite: &mut Sprite,
        tex: &mut dyn FnMut(i32, i32, i32, i32) -> Option<(Color, u16)>,
    ) {
        let rows = self.rows();
        if rows <= 0 {
            return;
        }
        for v in 0..rows {
            let (lx, ly) = self.pt(0.0, v as f32, rows as f32);
            let (rx, ry) = self.pt(1.0, v as f32, rows as f32);
            let dist = (rx - lx).hypot(ry - ly);
            let steps = ((dist * 2.0).ceil() as i32).max(1);

            for i in 0..=steps {
                let u = i as f32 / steps as f32;
                let x = (lx + (rx - lx) * u).round() as i32;
                let y = (ly + (ry - ly) * u).round() as i32;
                if let Some((color, depth)) = tex(i, v, x, y) {
                    if color != 0 {
                        sprite.set_pixel(x, y, color, depth);
                    }
                }
            }
        }
    }

    /// Fill with solid color and interpolated depth.
    pub fn fill(&self, sprite: &mut Sprite, color: Color) {
        let z_base = self.z_base;
        let z_top = self.z_top;
        self.paint(sprite, &mut |_u, v, _x, _y| {
            let rows = self.rows().max(1);
            let t = v as f32 / rows as f32;
            let depth = (z_top as f32 + (z_base as f32 - z_top as f32) * t).clamp(0.0, 65535.0) as u16;
            Some((color, depth))
        });
    }

    /// Fill with alpha-blended color (e.g. for shadows).
    pub fn fill_blended(&self, sprite: &mut Sprite, color: Color, alpha: f32) {
        let rows = self.rows();
        if rows <= 0 || alpha <= 0.0 {
            return;
        }
        for v in 0..rows {
            let (lx, ly) = self.pt(0.0, v as f32, rows as f32);
            let (rx, ry) = self.pt(1.0, v as f32, rows as f32);
            let dist = (rx - lx).hypot(ry - ly);
            let steps = ((dist * 2.0).ceil() as i32).max(1);

            for i in 0..=steps {
                let u = i as f32 / steps as f32;
                let x = (lx + (rx - lx) * u).round() as i32;
                let y = (ly + (ry - ly) * u).round() as i32;
                sprite.blend_pixel(x, y, color, 0, alpha);
            }
        }
    }
}

/// Create the left (facing SW) and right (facing SE) wall quads for a box.
pub fn box_faces(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z0: i32,
    z1: i32,
    ox: f32,
    oy: f32,
) -> (Quad, Quad) {
    // Left face: facing front-left (along y = y1, x from x0 to x1)
    let lf = Quad::new(
        project(x0, y1, z1 as f32, ox, oy),
        project(x1, y1, z1 as f32, ox, oy),
        project(x1, y1, z0 as f32, ox, oy),
        project(x0, y1, z0 as f32, ox, oy),
        z0,
        z1,
    );
    // Right face: facing front-right (along x = x1, y from y0 to y1)
    let rf = Quad::new(
        project(x1, y0, z1 as f32, ox, oy),
        project(x1, y1, z1 as f32, ox, oy),
        project(x1, y1, z0 as f32, ox, oy),
        project(x1, y0, z0 as f32, ox, oy),
        z0,
        z1,
    );
    (lf, rf)
}

/// Create the horizontal top quad for a box at height z.
pub fn box_top(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    ox: f32,
    oy: f32,
) -> Quad {
    Quad::new(
        project(x0, y0, z as f32, ox, oy),
        project(x1, y0, z as f32, ox, oy),
        project(x1, y1, z as f32, ox, oy),
        project(x0, y1, z as f32, ox, oy),
        z,
        z,
    )
}

/// Scanline polygon fill for arbitrary 2D convex/simple polygon with depth.
pub fn fill_polygon(
    sprite: &mut Sprite,
    pts: &[(f32, f32)],
    color: Color,
    depth: u16,
) {
    if pts.len() < 3 || color == 0 {
        return;
    }
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for &p in pts {
        min_y = min_y.min(p.1);
        max_y = max_y.max(p.1);
    }
    let y0 = (min_y.floor() as i32).max(0);
    let y1 = (max_y.ceil() as i32).min(sprite.h as i32);
    if y1 <= y0 {
        return;
    }

    let mut xs = [0.0f32; 16];
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        let mut n = 0;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                if n < xs.len() {
                    let t = (yc - a.1) / (b.1 - a.1);
                    xs[n] = a.0 + t * (b.0 - a.0);
                    n += 1;
                }
            }
        }
        if n < 2 {
            continue;
        }
        xs[..n].sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < n {
            let xa = (xs[i]).round() as i32;
            let xb = (xs[i + 1]).round() as i32;
            for x in xa..=xb {
                sprite.set_pixel(x, y, color, depth);
            }
            i += 2;
        }
    }
}

/// Scanline polygon fill with alpha blending (e.g. for angled shadows).
pub fn fill_polygon_blended(
    sprite: &mut Sprite,
    pts: &[(f32, f32)],
    color: Color,
    alpha: f32,
) {
    if pts.len() < 3 || alpha <= 0.0 {
        return;
    }
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for &p in pts {
        min_y = min_y.min(p.1);
        max_y = max_y.max(p.1);
    }
    let y0 = (min_y.floor() as i32).max(0);
    let y1 = (max_y.ceil() as i32).min(sprite.h as i32);
    if y1 <= y0 {
        return;
    }

    let mut xs = [0.0f32; 16];
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        let mut n = 0;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                if n < xs.len() {
                    let t = (yc - a.1) / (b.1 - a.1);
                    xs[n] = a.0 + t * (b.0 - a.0);
                    n += 1;
                }
            }
        }
        if n < 2 {
            continue;
        }
        xs[..n].sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < n {
            let xa = (xs[i]).round() as i32;
            let xb = (xs[i + 1]).round() as i32;
            for x in xa..=xb {
                sprite.blend_pixel(x, y, color, 0, alpha);
            }
            i += 2;
        }
    }
}

/// Draw a single 1-pixel Bresenham line.
pub fn draw_line(
    sprite: &mut Sprite,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Color,
    depth: u16,
) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        sprite.set_pixel(x, y, color, depth);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
