//! B67: a motion probe. For every moving thing the scene draws, the integer
//! pixel position it is drawn at this frame — the same projection and the same
//! truncation the draw code uses — so a harness can measure how evenly each of
//! them steps from one frame to the next.
//!
//! Nothing here runs unless a tool asks for it (`CITYLIVE_MOTION_TRACE` in the
//! wallpaper, or a test).

use super::Scene;
use crate::rail::Kind as RailKind;

/// One probed thing: its class, its index within the class and the pixel it is
/// drawn at.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub class: &'static str,
    pub idx: u32,
    pub x: i32,
    pub y: i32,
    /// The same position before the draw code truncates it.
    pub fx: f32,
    pub fy: f32,
}

impl Scene {
    /// Append the drawn position of every mover, car, walker and cloud.
    pub fn motion_probe(&self, out: &mut Vec<Probe>) {
        for (i, &(x, y)) in self.people_px.iter().enumerate() {
            let f = &self.people[i];
            let z = self.map.ground_px(f.x, f.y);
            let (fx, fy) = self.cam.project(f.x, f.y, z + f.lift as i32 * 2);
            out.push(Probe { class: "walker", idx: i as u32, x, y, fx, fy });
        }
        for (i, b) in self.traffic.iter().enumerate() {
            let z = self.map.ground_px(b.x, b.y) + b.lift as i32;
            let (sx, sy) = self.cam.project(b.x, b.y, z);
            out.push(Probe { class: "car", idx: i as u32, x: sx as i32, y: sy as i32, fx: sx, fy: sy });
        }
        for (i, b) in self.movers.iter().enumerate() {
            let z = if b.kind == RailKind::Plane || b.kind == RailKind::Aircraft {
                b.lift
            } else {
                self.map.ground_px(b.x, b.y)
            };
            let (sx, sy) = self.cam.project(b.x, b.y, z);
            let class = match b.kind {
                RailKind::Train => "train",
                RailKind::Tram => "tram",
                RailKind::Bus => "bus",
                RailKind::Ferry | RailKind::Boat | RailKind::Ship => "boat",
                RailKind::Plane | RailKind::Aircraft => "plane",
                RailKind::Crane => "crane",
                _ => "mover",
            };
            out.push(Probe { class, idx: i as u32, x: sx as i32, y: sy as i32, fx: sx, fy: sy });
        }
        for (i, c) in self.clouds.band.iter().enumerate() {
            let (x, y) = self.clouds.band_tl_pub(c);
            let (fx, fy) = self.clouds.band_tl_f(c);
            out.push(Probe { class: "cloud", idx: i as u32, x, y, fx, fy });
        }
        for (i, c) in self.clouds.lows.iter().enumerate() {
            let (x, y) = self.clouds.low_tl_pub(c, &self.cam, &self.map, &self.light);
            let (fx, fy) = self.clouds.low_tl_f(c, &self.cam, &self.map, &self.light);
            out.push(Probe { class: "cloud_shadow", idx: i as u32, x, y, fx, fy });
        }
    }
}
