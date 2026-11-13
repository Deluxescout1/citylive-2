//! Viewport maths: how big the world is for a given framebuffer, and what
//! integer upscale to use for a given screen.
//!
//! The camera is fixed by [`world_for_viewport`] so that the framebuffer is
//! exactly covered by tiles: the framebuffer's four corners land on the four
//! sides of the world rectangle in tile space, so there is no empty corner and
//! nothing is simulated that could never be seen.

use crate::MAX_LEVEL;
use crate::iso_consts::{HALF_H, HALF_W, HEIGHT_UNIT};

/// The tile rectangle that fills a framebuffer, plus the screen position of
/// tile `(0, 0)`'s top corner (the camera origin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Screen position of tile-space `(0, 0, 0)`.
    pub origin: (i32, i32),
    /// World width in tiles.
    pub w: i32,
    /// World height in tiles.
    pub h: i32,
}

impl Viewport {
    pub fn tiles(&self) -> i32 {
        self.w * self.h
    }
}

/// Tile rectangle that covers `fb_w x fb_h` exactly, with the camera fixed.
///
/// Derivation: with the 2:1 projection `sx = ox + (x - y) * 32`,
/// `sy = oy + (x + y) * 16`, the framebuffer rect maps to a 45-degree
/// parallelogram in tile space. Choosing `ox = W/2`, `oy = -W/4` puts the
/// framebuffer's top-left corner exactly on tile corner `(0, 15)` and the
/// bottom-right corner on `(29.9, 16.9)` for 960x540, so the tile-space
/// bounding box of the framebuffer starts at `(0, 0)`.
///
/// Raised terrain is drawn *higher* on screen, so a tile at level `L` shows up
/// as if the framebuffer were `L * HEIGHT_UNIT` further down. The world
/// therefore gets the maximum lift added to its bottom edge, otherwise the
/// bottom of the screen would need tiles beyond the world's far corner.
pub fn world_for_viewport(fb_w: i32, fb_h: i32) -> Viewport {
    let w = fb_w.max(16);
    let h = (fb_h.max(16)) + MAX_LEVEL as i32 * HEIGHT_UNIT;
    let ox = w / 2;
    let oy = -(w / 4);

    // The framebuffer rect in tile space, as an affine map of its corners.
    let inv_x = |sx: f32, sy: f32| {
        (sx - ox as f32) / (2.0 * HALF_W as f32) + (sy - oy as f32) / (2.0 * HALF_H as f32)
    };
    let inv_y = |sx: f32, sy: f32| {
        (sy - oy as f32) / (2.0 * HALF_H as f32) - (sx - ox as f32) / (2.0 * HALF_W as f32)
    };

    let corners = [
        (0.0, 0.0),
        (w as f32, 0.0),
        (0.0, h as f32),
        (w as f32, h as f32),
    ];
    // `h` here already includes the maximum terrain lift (see above).
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (sx, sy) in corners {
        let x = inv_x(sx, sy);
        let y = inv_y(sx, sy);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let tx0 = min_x.floor() as i32;
    let ty0 = min_y.floor() as i32;
    let tw = (max_x.ceil() as i32) - tx0;
    let th = (max_y.ceil() as i32) - ty0;

    // B61: a frame smaller than a world (a 7x5 spike, a tiny test window) used
    // to hand the generator fewer than its minimum tiles and abort the process.
    // The extra tiles are off the bottom-right corner of a frame that small, so
    // nothing on screen changes.
    let tw = tw.max(crate::terrain_gen::MIN_SIDE);
    let th = th.max(crate::terrain_gen::MIN_SIDE);

    // Re-base the world's tile (0,0) onto the bbox corner, keeping the same
    // screen placement.
    Viewport {
        origin: (ox + (tx0 - ty0) * HALF_W, oy + (tx0 + ty0) * HALF_H),
        w: tw,
        h: th,
    }
}

/// Inverse of the projection: which continuous tile position sits at screen
/// `(sx, sy)`. Used to put the first settlement in the middle of the frame (D35).
pub fn screen_to_tile(vp: &Viewport, sx: f32, sy: f32) -> (f32, f32) {
    let dx = sx - vp.origin.0 as f32;
    let dy = sy - vp.origin.1 as f32;
    (
        dx / (2.0 * HALF_W as f32) + dy / (2.0 * HALF_H as f32),
        dy / (2.0 * HALF_H as f32) - dx / (2.0 * HALF_W as f32),
    )
}

/// Integer upscale for an output: the `s` that keeps the art at 1× on a 1440p
/// screen. Bigger screens therefore show more city, not bigger pixels (P2).
pub fn pick_scale(out_w: i32, out_h: i32) -> (u32, i32, i32) {
    // D61: the art is 64-px tiles drawn at 1× on a 1440-row monitor, so
    // `s = max(1, round(H / 1440))` — 1440p and 1080p → 1×, 2160p → 2×.
    let s = ((out_h as f32 / 1440.0).round() as i32).max(1) as u32;
    (s, out_w / s as i32, out_h / s as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every framebuffer pixel must land inside the world rectangle. The inverse
    /// projection is affine, so checking all four extremes plus every pixel of
    /// the border is enough; we do every pixel anyway (it is cheap and literal).
    fn coverage(fb_w: i32, fb_h: i32) {
        let vp = world_for_viewport(fb_w, fb_h);
        let inv_x = |sx: f32, sy: f32| {
            (sx - vp.origin.0 as f32) / (2.0 * HALF_W as f32)
                + (sy - vp.origin.1 as f32) / (2.0 * HALF_H as f32)
        };
        let inv_y = |sx: f32, sy: f32| {
            (sy - vp.origin.1 as f32) / (2.0 * HALF_H as f32)
                - (sx - vp.origin.0 as f32) / (2.0 * HALF_W as f32)
        };
        for sy in 0..fb_h {
            for sx in 0..fb_w {
                let (fx, fy) = (sx as f32, sy as f32);
                let x = inv_x(fx, fy);
                let y = inv_y(fx, fy);
                assert!(
                    x >= 0.0 && x < vp.w as f32,
                    "{fb_w}x{fb_h}: pixel ({sx},{sy}) -> x={x} outside 0..{}",
                    vp.w
                );
                assert!(
                    y >= 0.0 && y < vp.h as f32,
                    "{fb_w}x{fb_h}: pixel ({sx},{sy}) -> y={y} outside 0..{}",
                    vp.h
                );
            }
        }
        assert!(vp.w > 0 && vp.h > 0);
    }

    #[test]
    fn covers_960x540() {
        coverage(960, 540);
        let vp = world_for_viewport(960, 540);
        // 960/64 + 540/32 tiles for the flat footprint, plus the lift margin.
        assert!(vp.w >= 38 && vp.w <= 41, "w = {}", vp.w);
        assert!(vp.h >= 38 && vp.h <= 41, "h = {}", vp.h);
    }

    /// D61: the same land on screen as the 32-px-at-2× engine — a 1440p monitor
    /// at 1× covers exactly the tiles that 720 internal rows of 32-px art did.
    #[test]
    fn d61_keeps_the_same_world_for_a_screen() {
        let (s, w, h) = pick_scale(7040, 1440);
        assert_eq!((s, w, h), (1, 7040, 1440));
        // The visible span in tiles is identical to the 32-px engine's
        // 3520x720 internal framebuffer at 2x (110 x 45 tiles of land).
        assert_eq!(w / crate::iso_consts::TILE_W, 3520 / 32);
        assert_eq!(h / crate::iso_consts::TILE_H, 720 / 16);
        // ...and so is the whole tile rectangle the world is generated for:
        // 162x162, measured from the 32-px engine before this change (the lift
        // margin doubles with the tile, so it costs the same in tiles).
        let now = world_for_viewport(w, h);
        assert_eq!((now.w, now.h), (162, 162));
    }

    #[test]
    fn covers_1440p_thirds() {
        coverage(853, 480);
    }

    #[test]
    fn covers_768p_halves() {
        coverage(683, 384);
    }

    #[test]
    fn covers_ultrawide() {
        coverage(1720, 720);
    }

    #[test]
    fn screen_to_tile_inverts_the_projection() {
        let vp = world_for_viewport(960, 540);
        for (sx, sy) in [(480.0, 270.0), (0.0, 0.0), (959.0, 539.0)] {
            let (tx, ty) = screen_to_tile(&vp, sx, sy);
            let bx = vp.origin.0 as f32 + (tx - ty) * HALF_W as f32;
            let by = vp.origin.1 as f32 + (tx + ty) * HALF_H as f32;
            assert!((bx - sx).abs() < 0.01 && (by - sy).abs() < 0.01);
        }
        // The middle of the frame must land inside the world.
        let (tx, ty) = screen_to_tile(&vp, 480.0, 270.0);
        assert!(tx > 0.0 && tx < vp.w as f32 && ty > 0.0 && ty < vp.h as f32);
    }

    #[test]
    fn pick_scale_matches_d61() {
        // s = max(1, round(H / 1440)): 64-px art at 1x on a 1440-row screen.
        assert_eq!(pick_scale(1920, 1080), (1, 1920, 1080));
        assert_eq!(pick_scale(2560, 1440), (1, 2560, 1440));
        assert_eq!(pick_scale(3840, 2160), (2, 1920, 1080));
        assert_eq!(pick_scale(1366, 768), (1, 1366, 768));
        assert_eq!(pick_scale(1280, 720), (1, 1280, 720));
        assert_eq!(pick_scale(3440, 1440), (1, 3440, 1440));
        // the owner's desk: 7040×1440 union → 1x → the native 7040×1440.
        assert_eq!(pick_scale(7040, 1440), (1, 7040, 1440));
    }
}
