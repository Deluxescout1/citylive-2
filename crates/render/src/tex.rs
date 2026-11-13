//! Per-pixel surface textures (D54/B32): the primitives that let terrain, roads,
//! walls and roofs carry real texture at 64-px tile detail instead of flat paint.
//!
//! Everything here is a *fill with a closure*: the geometry code knows the
//! shape, the texture code decides the colour of each pixel from its position
//! inside that shape. Two coordinate systems are offered, because the art needs
//! both:
//!
//! * **tile space** (`diamond_px`) — `u` runs along the tile's `x` axis and `v`
//!   along its `y` axis, both 0..1, so a paving joint, a crop row or a road
//!   marking lines up with the tile grid however the camera is placed;
//! * **face space** (`quad_px`) — `u` across a wall or roof from its top-left
//!   corner, `v` down it, so brick courses, siding boards and window grids are
//!   addressed in the building's own pixels.
//!
//! All textures are hashed from integer pixel positions: deterministic per tile,
//! no allocation, and a dirty-rect repaint produces exactly the same pixels as a
//! full rebuild.

use crate::framebuffer::{Framebuffer, Rect};
use crate::palette::{Color, TRANSPARENT};

/// Deterministic 0..1 hash of a pixel and a salt.
#[inline]
pub fn hash2(x: i32, y: i32, salt: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9)
        ^ (y as u32).wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2D);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Deterministic 0..1 hash of one integer (for picking among variants).
#[inline]
pub fn hash1(n: u32) -> f32 {
    hash2(n as i32, 0x5bd1, 0x9e37)
}

/// Per-pixel fill of a convex/simple polygon. The closure gets the pixel and
/// its normalised position over the polygon's bounding box.
pub fn fill_poly_px<F>(fb: &mut Framebuffer, pts: &[(f32, f32)], mut f: F)
where
    F: FnMut(i32, i32, f32, f32) -> Color,
{
    if pts.len() < 3 {
        return;
    }
    let bbox = Rect::from_points(pts);
    let clip = fb.clip();
    let y0 = bbox.y0.max(clip.y0);
    let y1 = bbox.y1.min(clip.y1);
    if y1 <= y0 {
        return;
    }
    let (bw, bh) = (bbox.width().max(1) as f32, bbox.height().max(1) as f32);
    let mut xs = [0f32; 16];
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        let mut n = 0usize;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            if ((a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc)) && n < xs.len() {
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
            let xa = ((xs[i] - 0.5).ceil() as i32).max(clip.x0);
            let xb = ((xs[i + 1] - 0.5).ceil() as i32).min(clip.x1);
            let v = (yc - bbox.y0 as f32) / bh;
            for x in xa..xb {
                let u = (x as f32 + 0.5 - bbox.x0 as f32) / bw;
                let c = f(x, y, u, v);
                if c != TRANSPARENT {
                    fb.put(x, y, c);
                }
            }
            i += 2;
        }
    }
}

/// Geometry of a tile diamond: its top corner and half-extents. `Camera::corners`
/// returns the tile's corners in tile order — `0 = (x, y)` which projects to the
/// *top* corner, `1 = (x+1, y)` right, `2 = (x+1, y+1)` bottom, `3 = (x, y+1)`
/// left — so the top corner plus the half-extents describe it completely.
#[inline]
fn diamond_geometry(p: &[(f32, f32); 4]) -> (f32, f32, f32, f32) {
    let (top, right, bottom, left) = (p[0], p[1], p[2], p[3]);
    let cx = (left.0 + right.0) * 0.5;
    let cy = (top.1 + bottom.1) * 0.5;
    let hw = ((right.0 - cx).abs().max(1.0)) as f32;
    let hh = ((bottom.1 - cy).abs().max(1.0)) as f32;
    (top.0, top.1, hw, hh)
}

/// Tile-space `(u, v)` of a pixel in a diamond: `u` runs along the tile's `x`
/// axis (top corner → right corner) and `v` along `y` (top corner → left).
#[inline]
fn diamond_uv(dx: f32, dy: f32, hw: f32, hh: f32) -> (f32, f32) {
    (0.5 * (dx / hw + dy / hh), 0.5 * (dy / hh - dx / hw))
}

/// Fill a tile's top face (a 2:1 diamond, corners in `p` as returned by
/// [`crate::iso::Camera::corners`]) with tile-space coordinates: `u` 0..1 along
/// the tile's `x` axis, `v` 0..1 along `y`.
pub fn diamond_px<F>(fb: &mut Framebuffer, p: &[(f32, f32); 4], mut f: F)
where
    F: FnMut(i32, i32, f32, f32) -> Color,
{
    let (ox, oy, hw, hh) = diamond_geometry(p);
    let clip = fb.clip();
    let y0 = (oy.floor() as i32).max(clip.y0);
    let y1 = ((oy + 2.0 * hh).ceil() as i32).min(clip.y1);
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - oy;
        let half = hw * (1.0 - ((dy - hh) / hh).abs().min(1.0));
        if half < 0.5 {
            continue;
        }
        let xa = ((ox - half).floor() as i32).max(clip.x0);
        let xb = ((ox + half).ceil() as i32 + 1).min(clip.x1);
        for x in xa..xb {
            let (u, v) = diamond_uv(x as f32 + 0.5 - ox, dy, hw, hh);
            if !(-0.07..=1.07).contains(&u) || !(-0.07..=1.07).contains(&v) {
                continue;
            }
            let c = f(x, y, u, v);
            if c != TRANSPARENT {
                fb.put(x, y, c);
            }
        }
    }
}

/// Fill one triangle of a tile's top face, with tile-space coordinates.
///
/// A tile on a slope has four different corner heights, so its top face is a
/// general quadrilateral, not the parallelogram a flat tile is: the triangle is
/// rasterised with barycentric coordinates and `(u, v)` interpolated from the
/// tile's own corners (`0=(0,0) 1=(1,0) 2=(1,1) 3=(0,1)`). That is exact for
/// ramps as well as flats, and two triangles with a hair of overlap leave no
/// seam — a seam here shows the sky through a hillside.
pub fn diamond_tri_px<F>(
    fb: &mut Framebuffer,
    p: &[(f32, f32); 4],
    tri: (usize, usize, usize),
    mut f: F,
) where
    F: FnMut(i32, i32, f32, f32) -> Color,
{
    const UV: [(f32, f32); 4] = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let (a, b, c) = (p[tri.0], p[tri.1], p[tri.2]);
    let (ua, ub, uc) = (UV[tri.0], UV[tri.1], UV[tri.2]);
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area.abs() < 1e-4 {
        return;
    }
    let clip = fb.clip();
    let bbox = Rect::from_points(&[a, b, c]).expanded(1).intersect(&clip);
    if bbox.is_empty() {
        return;
    }
    for y in bbox.y0..bbox.y1 {
        let py = y as f32 + 0.5;
        for x in bbox.x0..bbox.x1 {
            let px = x as f32 + 0.5;
            let w0 = ((b.0 - px) * (c.1 - py) - (b.1 - py) * (c.0 - px)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (c.1 - py) * (a.0 - px)) / area;
            let w2 = 1.0 - w0 - w1;
            // A hair of overdraw (about 1.5 px) closes the seam where two tiles
            // at different levels meet: without it the sky shows through a
            // hillside in 1-px diagonal dashes.
            if w0 < -0.055 || w1 < -0.055 || w2 < -0.055 {
                continue;
            }
            let u = w0 * ua.0 + w1 * ub.0 + w2 * uc.0;
            let v = w0 * ua.1 + w1 * ub.1 + w2 * uc.1;
            let col = f(x, y, u, v);
            if col != TRANSPARENT {
                fb.put(x, y, col);
            }
        }
    }
}

/// A wall face: a parallelogram with vertical sides, filled completely.
///
/// B43c: [`quad_px`] walks rows and tests both vertical edges, which drops the
/// two wedges at the slanted top and bottom — everything when the wall is lower
/// than the slant is wide, so the faces of a tall box came out as a sliver.
/// This walks the columns instead (`u` runs from `tl` to `tr`, `v` down).
pub fn wall_quad_px<F>(fb: &mut Framebuffer, tl: (f32, f32), tr: (f32, f32), bl: (f32, f32), br: (f32, f32), mut f: F)
where
    F: FnMut(i32, i32, f32, f32) -> Color,
{
    let clip = fb.clip();
    if (tr.0 - tl.0).abs() < 0.5 {
        return;
    }
    let flip = tr.0 < tl.0;
    let (lt_, lb_, rt_, rb_) = if flip { (tr, br, tl, bl) } else { (tl, bl, tr, br) };
    let dx = rt_.0 - lt_.0;
    let xa = ((lt_.0 - 0.5).ceil() as i32).max(clip.x0);
    let xb = ((rt_.0 - 0.5).ceil() as i32).min(clip.x1);
    for x in xa..xb {
        let t = ((x as f32 + 0.5 - lt_.0) / dx).clamp(0.0, 1.0);
        let yt = lt_.1 + (rt_.1 - lt_.1) * t;
        let yb = lb_.1 + (rb_.1 - lb_.1) * t;
        let ya = ((yt - 0.5).ceil() as i32).max(clip.y0);
        let yz = ((yb - 0.5).ceil() as i32).min(clip.y1);
        let span = (yb - yt).max(0.001);
        let u = if flip { 1.0 - t } else { t };
        for y in ya..yz {
            let v = ((y as f32 + 0.5 - yt) / span).clamp(0.0, 1.0);
            let c = f(x, y, u, v);
            if c != TRANSPARENT {
                fb.put(x, y, c);
            }
        }
    }
}

/// Fill a quad (wall face, roof slope, road corridor) with `u` 0..1 across the
/// top edge and `v` 0..1 down the left edge.
pub fn quad_px<F>(fb: &mut Framebuffer, tl: (f32, f32), tr: (f32, f32), bl: (f32, f32), br: (f32, f32), mut f: F)
where
    F: FnMut(i32, i32, f32, f32) -> Color,
{
    let bbox = Rect::from_points(&[tl, tr, bl, br]);
    let clip = fb.clip();
    let y0 = bbox.y0.max(clip.y0);
    let y1 = bbox.y1.min(clip.y1);
    let (h0, h1) = (bl.1 - tl.1, br.1 - tr.1);
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        // Left and right edge positions on this row.
        let lt = if h0.abs() > 0.001 { (yc - tl.1) / h0 } else { 0.0 };
        let rt = if h1.abs() > 0.001 { (yc - tr.1) / h1 } else { 0.0 };
        // A hair of slack: a face that ends exactly on a pixel centre must still
        // paint that pixel, or a step shows a 1-px line of sky.
        if !(-0.03..=1.03).contains(&lt) || !(-0.03..=1.03).contains(&rt) {
            continue;
        }
        let lx = tl.0 + (bl.0 - tl.0) * lt;
        let rx = tr.0 + (br.0 - tr.0) * rt;
        if rx < lx {
            continue;
        }
        let xa = ((lx - 0.5).floor() as i32).max(clip.x0);
        let xb = ((rx - 0.5).ceil() as i32 + 1).min(clip.x1);
        let v = (lt + rt) * 0.5;
        let span = rx - lx;
        for x in xa..xb {
            let u = if span > 0.5 { (x as f32 + 0.5 - lx) / span } else { 0.5 };
            let c = f(x, y, u, v);
            if c != TRANSPARENT {
                fb.put(x, y, c);
            }
        }
    }
}

/// Per-pixel grain in two steps either side of `base` (ballast, timber, stone).
#[inline]
pub fn grain_px(base: Color, px: i32, py: i32, salt: u32) -> Color {
    let h = hash2(px, py, salt);
    if h < 0.30 {
        crate::palette::shade(base, 0.90)
    } else if h > 0.72 {
        crate::palette::shade(base, 1.10)
    } else {
        base
    }
}

/// A short line of `len` pixels with a 1-px shadow side, used for grass blades,
/// twigs and grass tufts: cheap and reads at 1×.
pub fn tuft(fb: &mut Framebuffer, x: i32, y: i32, len: i32, lean: i32, c: Color) {
    for i in 0..len {
        fb.put(x + (i * lean) / len.max(1), y - i, c);
    }
}

/// A filled ellipse (ponds, tree canopies, manhole covers, lamp pools).
pub fn ellipse(fb: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, c: Color) {
    let (x0, y0) = ((cx - rx).floor() as i32, (cy - ry).floor() as i32);
    let (x1, y1) = ((cx + rx).ceil() as i32, (cy + ry).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - cx) / rx.max(0.5);
            let dy = (y as f32 + 0.5 - cy) / ry.max(0.5);
            if dx * dx + dy * dy <= 1.0 {
                fb.put(x, y, c);
            }
        }
    }
}

/// A filled ellipse with a per-pixel colour (canopy shading, rock facets).
pub fn ellipse_px<F>(fb: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, mut f: F)
where
    F: FnMut(i32, i32, f32) -> Color,
{
    let (x0, y0) = ((cx - rx).floor() as i32, (cy - ry).floor() as i32);
    let (x1, y1) = ((cx + rx).ceil() as i32, (cy + ry).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - cx) / rx.max(0.5);
            let dy = (y as f32 + 0.5 - cy) / ry.max(0.5);
            let d2 = dx * dx + dy * dy;
            if d2 <= 1.0 {
                let c = f(x, y, d2.sqrt());
                if c != TRANSPARENT {
                    fb.put(x, y, c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso::Camera;
    use crate::palette::rgb;

    #[test]
    fn diamond_covers_the_tile_and_uses_tile_space() {
        let cam = Camera::new(100, 100);
        let p = cam.corners(0, 0, [0.0; 4]);
        let mut fb = Framebuffer::new(200, 200);
        let mut seen = 0;
        let mut centre = (0.0f32, 0.0f32);
        diamond_px(&mut fb, &p, |_, _, u, v| {
            seen += 1;
            centre.0 += u;
            centre.1 += v;
            rgb(1, 2, 3)
        });
        // A 64x32 diamond has 64*32/2 = 1024 pixels.
        assert!(seen > 960 && seen < 1100, "diamond painted {seen} px");
        // The mean of a symmetric parameterisation is the middle of the tile.
        let n = seen as f32;
        assert!((centre.0 / n - 0.5).abs() < 0.02);
        assert!((centre.1 / n - 0.5).abs() < 0.02);
        // A pixel outside the diamond stays transparent.
        assert_eq!(fb.px()[0], TRANSPARENT);
    }

    #[test]
    fn quad_space_runs_across_and_down() {
        let mut fb = Framebuffer::new(40, 40);
        let mut maxu = 0.0f32;
        let mut maxv = 0.0f32;
        quad_px(&mut fb, (0.0, 0.0), (20.0, 0.0), (0.0, 20.0), (20.0, 20.0), |_, _, u, v| {
            maxu = maxu.max(u);
            maxv = maxv.max(v);
            rgb(9, 9, 9)
        });
        assert!(maxu > 0.9 && maxv > 0.9);
        assert_eq!(fb.px()[0], rgb(9, 9, 9));
        assert_eq!(fb.px()[21 * 40 + 21], TRANSPARENT);
    }
}

#[cfg(test)]
mod tiling_tests {
    use super::*;
    use crate::iso::Camera;
    use crate::palette::rgb;

    #[test]
    fn adjacent_diamonds_leave_no_gap() {
        let cam = Camera::new(100, 20);
        let mut fb = Framebuffer::new(260, 160);
        for y in 0..4 {
            for x in 0..4 {
                let p = cam.corners(x, y, [0.0; 4]);
                for tri in [(0usize, 1usize, 2usize), (0, 2, 3)] {
                    diamond_tri_px(&mut fb, &p, tri, |_, _, _, _| rgb(200, 100, 50));
                }
            }
        }
        // The 4x4 block's interior must be fully painted. The block covers the
        // diamond (100,20) (228,84) (100,148) (28,84), so this rectangle is well
        // inside it.
        let mut gaps = 0;
        for y in 60..104 {
            for x in 60..140 {
                if fb.px()[y as usize * 260 + x as usize] == TRANSPARENT {
                    gaps += 1;
                }
            }
        }
        for y in 56..76 {
            let mut row = String::new();
            for x in 56..144 {
                row.push(if fb.px()[y as usize * 260 + x as usize] == TRANSPARENT { '.' } else { '#' });
            }
            eprintln!("{y:>4} {row}");
        }
        assert_eq!(gaps, 0, "{gaps} transparent pixels between tiles");
    }
}
