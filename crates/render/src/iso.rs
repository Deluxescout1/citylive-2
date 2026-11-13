//! Isometric projection and tile geometry.
//!
//! Shared constants come from `map::iso_consts` so the world-size maths and the
//! renderer can never disagree. Everything here works in screen pixels; tile
//! space is continuous and `z` is in pixels (`level * HEIGHT_UNIT`).

use crate::framebuffer::Rect;
pub use map::iso_consts::HEIGHT_UNIT;
use map::iso_consts::{HALF_H, HALF_W};

pub type Pt = (f32, f32);

/// Level (as returned by [`map::TileMap::corner_levels`]) to screen pixels.
#[inline]
pub fn level_px(level: f32) -> i32 {
    (level * HEIGHT_UNIT as f32).round() as i32
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Screen position of tile-space `(0, 0, z = 0)`.
    pub ox: i32,
    pub oy: i32,
}

impl Camera {
    pub const fn new(ox: i32, oy: i32) -> Self {
        Self { ox, oy }
    }

    /// The camera the world was generated for (P3).
    pub fn for_viewport(vp: &map::Viewport) -> Self {
        Self::new(vp.origin.0, vp.origin.1)
    }

    #[inline]
    pub fn project(&self, x: f32, y: f32, z: i32) -> Pt {
        (
            self.ox as f32 + (x - y) * HALF_W as f32,
            self.oy as f32 + (x + y) * HALF_H as f32 - z as f32,
        )
    }

    #[inline]
    pub fn project_i(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        (self.ox + (x - y) * HALF_W, self.oy + (x + y) * HALF_H - z)
    }

    /// The four screen corners of tile `(x, y)`'s top face, from per-corner
    /// levels (a ramp has different levels per corner).
    #[inline]
    pub fn corners(&self, x: i32, y: i32, levels: [f32; 4]) -> [Pt; 4] {
        [
            self.project(x as f32, y as f32, level_px(levels[0])),
            self.project(x as f32 + 1.0, y as f32, level_px(levels[1])),
            self.project(x as f32 + 1.0, y as f32 + 1.0, level_px(levels[2])),
            self.project(x as f32, y as f32 + 1.0, level_px(levels[3])),
        ]
    }
}

/// Screen bounds of a tile column: top face at the tile's highest corner, down
/// to level 0. Conservative (used for culling and dirty rects).
pub fn tile_bounds(cam: &Camera, x: i32, y: i32, levels: [f32; 4]) -> Rect {
    let top = cam.corners(x, y, levels);
    let bottom = cam.corners(x, y, [0.0; 4]);
    let mut r = Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for p in top.iter().chain(bottom.iter()) {
        r.x0 = r.x0.min(p.0.floor() as i32);
        r.y0 = r.y0.min(p.1.floor() as i32);
        r.x1 = r.x1.max(p.0.ceil() as i32);
        r.y1 = r.y1.max(p.1.ceil() as i32);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_is_2_to_1() {
        let cam = Camera::new(100, 100);
        let a = cam.project(0.0, 0.0, 0);
        let b = cam.project(1.0, 0.0, 0);
        let c = cam.project(0.0, 1.0, 0);
        assert_eq!(b.0 - a.0, 32.0);
        assert_eq!(b.1 - a.1, 16.0);
        assert_eq!(c.0 - a.0, -32.0);
        assert_eq!(c.1 - a.1, 16.0);
    }

    #[test]
    fn diamond_is_64_by_32() {
        let cam = Camera::new(0, 0);
        let d = cam.corners(5, 5, [1.0; 4]);
        let r = Rect::from_points(&d);
        assert_eq!((r.width(), r.height()), (64, 32));
    }

    #[test]
    fn ramp_corners_differ() {
        let cam = Camera::new(0, 0);
        // Rising toward +x: corners 0 and 3 are a level lower. The step is
        // HALF_H - HEIGHT_UNIT pixels, so the top face really is a slope.
        let p = cam.corners(0, 0, [1.0, 2.0, 2.0, 1.0]);
        let dy = p[1].1 - p[0].1;
        assert_eq!(
            dy,
            (HALF_H - HEIGHT_UNIT) as f32,
            "ramp must not be horizontal"
        );
        assert_eq!(p[2].1 - p[3].1, dy);
    }
}
