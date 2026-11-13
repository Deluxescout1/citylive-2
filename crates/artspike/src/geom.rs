//! Iso projection and textured-quad drawing for the art spike.
//!
//! Everything is parameterized by the tile width in pixels (`TW`): 32 for the
//! classic engine scale (drawn at 2×), 64 for the high-detail scale (1×).
//! Tile space is continuous; `z` is screen pixels above the ground plane.

use render::framebuffer::Rect;
use render::{Framebuffer, palette::Color};

/// Tile geometry for one art resolution.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    /// Tile diamond width (32 or 64).
    pub tw: i32,
    /// Detail multiplier: tw / 32 (1 or 2). Draw code uses it to decide how
    /// much texture fits.
    pub s: i32,
    /// Screen position of tile-space (0, 0, z=0).
    pub ox: i32,
    pub oy: i32,
}

impl Grid {
    pub fn new(tw: i32, ox: i32, oy: i32) -> Self {
        Grid { tw, s: tw / 32, ox, oy }
    }

    #[inline]
    pub fn hw(&self) -> i32 { self.tw / 2 }
    #[inline]
    pub fn hh(&self) -> i32 { self.tw / 4 }

    /// Project tile-space (x, y, z px) to screen.
    #[inline]
    pub fn p(&self, x: f32, y: f32, z: i32) -> (f32, f32) {
        (
            self.ox as f32 + (x - y) * self.hw() as f32,
            self.oy as f32 + (x + y) * self.hh() as f32 - z as f32,
        )
    }

    /// Integer variant.
    #[inline]
    pub fn pi(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        (
            self.ox + (x - y) * self.hw(),
            self.oy + (x + y) * self.hh() - z,
        )
    }

    /// Float tile coords in, rounded screen px out.
    #[inline]
    pub fn pi_f(&self, x: f32, y: f32, z: i32) -> (i32, i32) {
        let (sx, sy) = self.p(x, y, z);
        (sx.round() as i32, sy.round() as i32)
    }
}

/// A screen-space quadrilateral walked in rows, with a (u, v) parameter:
/// `v` runs top→bottom along both side edges, `u` left→right across the row.
/// Faces, roofs and road corridors are all drawn through this one primitive,
/// with a closure deciding the colour of each pixel — that closure is where
/// the procedural texture (brick courses, siding, glass grid, markings) lives.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub tl: (f32, f32),
    pub tr: (f32, f32),
    pub br: (f32, f32),
    pub bl: (f32, f32),
}

impl Quad {
    pub fn new(tl: (f32, f32), tr: (f32, f32), br: (f32, f32), bl: (f32, f32)) -> Self {
        Quad { tl, tr, br, bl }
    }

    /// Number of v rows (longest side edge, rounded).
    pub fn rows(&self) -> i32 {
        let dl = (self.bl.1 - self.tl.1).hypot(self.bl.0 - self.tl.0);
        let dr = (self.br.1 - self.tr.1).hypot(self.br.0 - self.tr.0);
        dl.max(dr).round() as i32 + 1
    }

    /// The point at (u, v), v in rows.
    #[inline]
    pub fn pt(&self, u: f32, v: f32, rows: f32) -> (f32, f32) {
        let t = if rows > 0.0 { v / rows } else { 0.0 };
        let lx = self.tl.0 + (self.bl.0 - self.tl.0) * t;
        let ly = self.tl.1 + (self.bl.1 - self.tl.1) * t;
        let rx = self.tr.0 + (self.br.0 - self.tr.0) * t;
        let ry = self.tr.1 + (self.br.1 - self.tr.1) * t;
        (lx + (rx - lx) * u, ly + (ry - ly) * u)
    }

    /// Paint every pixel, colour from `tex(upx, vpx, x, y)` where `upx` is the
    /// pixel index along the row and `vpx` the row index (both local, so
    /// sub-quads get their own 0-based coordinates for crisp alignment).
    /// TRANSPARENT (0) means "skip this pixel". Rows outside the clip are
    /// stepped over cheaply (whole-corridor paints stay constant-cost).
    pub fn paint(&self, fb: &mut Framebuffer, tex: &mut dyn FnMut(i32, i32, i32, i32) -> Color) {
        let rows = self.rows();
        if rows <= 0 {
            return;
        }
        let clip = fb.clip();
        for v in 0..rows {
            let (lx, ly) = self.pt(0.0, v as f32, rows as f32);
            let (rx, ry) = self.pt(1.0, v as f32, rows as f32);
            let steps = ((rx - lx).abs().max((ry - ly).abs()).round() as i32).max(1);
            // param range of the row that lies inside the clip
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            if (rx - lx).abs() > 0.001 {
                let mut a = (clip.x0 as f32 - lx) / (rx - lx);
                let mut b = (clip.x1 as f32 - 1.0 - lx) / (rx - lx);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
            }
            if (ry - ly).abs() > 0.001 {
                let mut a = (clip.y0 as f32 - ly) / (ry - ly);
                let mut b = (clip.y1 as f32 - 1.0 - ly) / (ry - ly);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
            }
            if t1 < t0 {
                continue;
            }
            let i0 = (t0 * steps as f32).floor() as i32;
            let i1 = (t1 * steps as f32).ceil() as i32;
            for i in i0..=i1.min(steps) {
                let u = i as f32 / steps as f32;
                let x = (lx + (rx - lx) * u).round() as i32;
                let y = (ly + (ry - ly) * u).round() as i32;
                let c = tex(i, v, x, y);
                if c != 0 {
                    fb.put(x, y, c);
                }
            }
        }
    }

    /// Solid fill.
    pub fn fill(&self, fb: &mut Framebuffer, c: Color) {
        self.paint(fb, &mut |_, _, _, _| c);
    }

    /// Alpha-blended solid fill.
    pub fn fill_blended(&self, fb: &mut Framebuffer, c: Color, cov: f32) {
        let rows = self.rows();
        if rows <= 0 || cov <= 0.0 {
            return;
        }
        let clip = fb.clip();
        for v in 0..rows {
            let (lx, ly) = self.pt(0.0, v as f32, rows as f32);
            let (rx, ry) = self.pt(1.0, v as f32, rows as f32);
            let steps = ((rx - lx).abs().max((ry - ly).abs()).round() as i32).max(1);
            let (mut t0, mut t1) = (0.0f32, 1.0f32);
            if (rx - lx).abs() > 0.001 {
                let mut a = (clip.x0 as f32 - lx) / (rx - lx);
                let mut b = (clip.x1 as f32 - 1.0 - lx) / (rx - lx);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
            }
            if (ry - ly).abs() > 0.001 {
                let mut a = (clip.y0 as f32 - ly) / (ry - ly);
                let mut b = (clip.y1 as f32 - 1.0 - ly) / (ry - ly);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
            }
            if t1 < t0 {
                continue;
            }
            let i0 = (t0 * steps as f32).floor() as i32;
            let i1 = (t1 * steps as f32).ceil() as i32;
            for i in i0..=i1.min(steps) {
                let u = i as f32 / steps as f32;
                let x = (lx + (rx - lx) * u).round() as i32;
                let y = (ly + (ry - ly) * u).round() as i32;
                fb.put_blended(x, y, c, cov);
            }
        }
    }

    /// The four screen corners of tile `(x, y)`'s top face, from per-corner
    /// length of the top edge in px (for pixel-addressed sub-quads).
    pub fn top_len(&self) -> i32 {
        (self.tr.0 - self.tl.0).hypot(self.tr.1 - self.tl.1).round() as i32
    }

    /// A sub-quad addressed in face pixels: `u0px..u1px` across, `v0px..v1px`
    /// down (v in the same units as `rows`).
    pub fn sub_px(&self, u0px: i32, v0px: i32, u1px: i32, v1px: i32, rows: f32) -> Quad {
        let w = self.top_len().max(1) as f32;
        let u0 = u0px as f32 / w;
        let u1 = u1px as f32 / w;
        Quad::new(
            self.pt(u0, v0px as f32, rows),
            self.pt(u1, v0px as f32, rows),
            self.pt(u1, v1px as f32, rows),
            self.pt(u0, v1px as f32, rows),
        )
    }

    /// A sub-quad at u0..u1, v0..v1 (v in the same pixel units as rows).
    pub fn sub(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> Quad {
        let rows = self.rows() as f32;
        Quad::new(
            self.pt(u0, v0, rows),
            self.pt(u1, v0, rows),
            self.pt(u1, v1, rows),
            self.pt(u0, v1, rows),
        )
    }
}

/// The ground diamond of tile (x, y).
pub fn tile_quad(g: &Grid, x: i32, y: i32) -> Quad {
    let z = 0;
    Quad::new(
        g.p(x as f32, y as f32, z),
        g.p(x as f32 + 1.0, y as f32, z),
        g.p(x as f32 + 1.0, y as f32 + 1.0, z),
        g.p(x as f32, y as f32 + 1.0, z),
    )
}

/// A corridor quad: the rectangle of tile space x0..x1, y0..y1 at height z.
/// Roads/sidewalks/medians are drawn as whole corridors so markings run
/// continuously instead of per tile.
pub fn corridor_quad(g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, z: i32) -> Quad {
    Quad::new(
        g.p(x0, y0, z),
        g.p(x1, y0, z),
        g.p(x1, y1, z),
        g.p(x0, y1, z),
    )
}

/// The two visible wall faces of an axis-aligned box: `Some((left, right))`.
/// Left = the +y face (screen-left, lighter); right = the +x face (darker).
pub fn box_faces(
    g: &Grid,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z0: i32,
    z1: i32,
) -> (Quad, Quad) {
    let left = Quad::new(
        g.p(x0, y1, z1),
        g.p(x1, y1, z1),
        g.p(x1, y1, z0),
        g.p(x0, y1, z0),
    );
    let right = Quad::new(
        g.p(x1, y0, z1),
        g.p(x1, y1, z1),
        g.p(x1, y1, z0),
        g.p(x1, y0, z0),
    );
    (left, right)
}

/// The top face (diamond) of an axis-aligned box.
pub fn box_top(g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, z: i32) -> Quad {
    Quad::new(
        g.p(x0, y0, z),
        g.p(x1, y0, z),
        g.p(x1, y1, z),
        g.p(x0, y1, z),
    )
}

/// Conservative screen bounds of a quad (for culling).
pub fn quad_bounds(q: &Quad) -> Rect {
    Rect::from_points(&[q.tl, q.tr, q.br, q.bl])
}

/// Alpha-blended scanline fill of a simple polygon.
pub fn fill_poly_blended(fb: &mut Framebuffer, pts: &[(f32, f32)], c: Color, cov: f32) {
    if pts.len() < 3 || cov <= 0.0 {
        return;
    }
    let bbox = Rect::from_points(pts);
    let clip = fb.clip();
    let y0 = bbox.y0.max(clip.y0);
    let y1 = bbox.y1.min(clip.y1);
    if y1 <= y0 {
        return;
    }

    let mut xs = [0f32; 16];
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        let mut n = 0usize;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            let crosses = (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc);
            if crosses && n < xs.len() {
                let t = (yc - a.1) / (b.1 - a.1);
                xs[n] = a.0 + t * (b.0 - a.0);
                n += 1;
            }
        }
        if n < 2 {
            continue;
        }
        xs[..n].sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
        let mut i = 0;
        while i + 1 < n {
            let xa = (xs[i] - 0.5).ceil() as i32;
            let xb = (xs[i + 1] - 0.5).ceil() as i32;
            let a = xa.max(clip.x0);
            let b = xb.min(clip.x1);
            for x in a..b {
                fb.put_blended(x, y, c, cov);
            }
            i += 2;
        }
    }
}
