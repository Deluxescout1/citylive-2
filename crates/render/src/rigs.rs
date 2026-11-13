//! B43c: the moving half of a building site. Tower and mobile cranes, the
//! excavator, concrete-mixer and delivery trucks, the site cabin, the portable
//! toilet, the skip. They are drawn per frame (the baked layer holds only the
//! plot itself), depth-tested against the static layer so a building in front
//! hides them, and their pose is a pure function of one work clock, so a site
//! that is idle (night) changes no pixel and costs nothing.

use crate::actors::{actor_key, static_key};
use crate::city::{CityObj, ObjKind, Stage};
use crate::framebuffer::{Framebuffer, Rect};
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::light::ActorShadow;
use crate::palette::{self, pal, Color};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Tower,
    Mobile,
    Excavator,
    Mixer,
    Truck,
    Cabin,
    Toilet,
    Skip,
}

#[derive(Clone, Debug)]
pub struct Rig {
    pub kind: Kind,
    /// Base position in tiles and the ground height in px.
    pub x: f32,
    pub y: f32,
    pub z: i32,
    /// The finished building's height in px.
    pub h: i32,
    pub seed: u32,
    /// Frontage axis (unit) and the direction from the street into the lot.
    pub axis: (f32, f32),
    pub inward: (f32, f32),
    /// Centre of the lot.
    pub centre: (f32, f32),
    /// Crane reach in tiles.
    pub reach: f32,
    /// A tower crane in its last step: the jib is down, the counter-jib and
    /// the mast wait for the building to be finished.
    pub late: bool,
    /// Depth key the rig is drawn with.
    pub key: u16,
    /// Everything the rig can ever cover (for culling), in framebuffer px.
    pub bounds: Rect,
    /// The state last marked dirty, and the box it covered.
    pub sig: u64,
    pub last: Rect,
}

fn sfv(seed: u32) -> f32 {
    (seed % 997) as f32 * 0.37
}

/// Building-site working hours (the sim's own schedule): 07:00-18:00 Monday
/// to Friday, 07:00-13:00 Saturday, nobody on Sunday. `weekday` 0 = Sunday.
pub fn site_hours(hour: f32, weekday: i32) -> bool {
    match weekday {
        0 => false,
        6 => (7.0..13.0).contains(&hour),
        _ => (7.0..18.0).contains(&hour),
    }
}

/// The rigs a set of plots needs right now. `map` keeps parked plant off the
/// lanes: a prop whose verge spot is a lane moves inside the lot.
pub fn rigs_for(objs: &[CityObj], cam: &Camera, map: &map::TileMap) -> Vec<Rig> {
    let mut out = Vec::new();
    for o in objs {
        if !o.is_site() || o.stage < Stage::Graded {
            continue;
        }
        let h = o.wall_height();
        let z = o.level as i32 * HEIGHT_UNIT;
        let (lx, ly, lw, lh) = (o.x as f32, o.y as f32, o.w as f32, o.h as f32);
        let centre = (lx + lw * 0.5, ly + lh * 0.5);
        let (e, out_v, axis, span) = match o.front % 4 {
            0 => ((lx + lw, centre.1), (1.0, 0.0), (0.0, 1.0), lh),
            1 => ((centre.0, ly + lh), (0.0, 1.0), (1.0, 0.0), lw),
            2 => ((lx, centre.1), (-1.0, 0.0), (0.0, 1.0), lh),
            _ => ((centre.0, ly), (0.0, -1.0), (1.0, 0.0), lw),
        };
        let inward = (-out_v.0, -out_v.1);
        let at = |along: f32, off: f32| {
            (
                e.0 + axis.0 * along + out_v.0 * off,
                e.1 + axis.1 * along + out_v.1 * off,
            )
        };
        let base_key = static_key(o.diagonal(), true);
        // Outside the lot on its own verge when that is not a lane, else just
        // inside the front edge.
        let blocked = |p: (f32, f32)| {
            let t = map.at(p.0.floor() as i32, p.1.floor() as i32);
            t.kind.is_lane() || t.kind.is_water()
        };
        let place = |along: f32, off: f32| {
            let p = at(along, off);
            if blocked(p) { (at(along, -0.26), true) } else { (p, false) }
        };
        let mut push = |kind: Kind, p: (f32, f32), reach: f32, key: u16, pad: (i32, i32, i32, i32)| {
            let s = cam.project(p.0, p.1, z);
            let b = Rect::new(
                s.0 as i32 - pad.0,
                s.1 as i32 - pad.1,
                s.0 as i32 + pad.2,
                s.1 as i32 + pad.3,
            );
            out.push(Rig {
                kind,
                x: p.0,
                y: p.1,
                z,
                h,
                seed: o.seed,
                axis,
                inward,
                centre,
                reach,
                late: false,
                key,
                bounds: b,
                sig: u64::MAX,
                last: b,
            });
        };
        let along_key = |p: (f32, f32)| actor_key(p.0.floor() as i32, p.1.floor() as i32);
        let stage = o.stage;
        let key_for = |p: (f32, f32), inside: bool| if inside { base_key } else { along_key(p) };
        // The digger on the rim of the pit.
        if stage == Stage::Graded {
            let (p, inside) = place(span * 0.22, 0.32);
            push(Kind::Excavator, p, 0.0, key_for(p, inside), (36, 50, 36, 16));
        }
        // Trucks stand just inside the front edge of the lot.
        if stage == Stage::Foundation {
            let p = at(0.0, -0.26);
            push(Kind::Mixer, p, 0.0, base_key, (90, 54, 90, 20));
        }
        if stage == Stage::Roofed {
            let p = at(0.0, -0.26);
            push(Kind::Truck, p, 0.0, base_key, (90, 54, 90, 20));
        }
        // Site facilities from the foundation on (the sim's material pile
        // stands at +0.9 along the frontage, so these keep to the other side).
        if stage >= Stage::Foundation && span >= 2.0 {
            let (p, inside) = place(-span * 0.2, 0.42);
            push(Kind::Cabin, p, 0.0, key_for(p, inside), (16, 28, 16, 8));
            let (p, inside) = place(-span * 0.49, 0.38);
            push(Kind::Toilet, p, 0.0, key_for(p, inside), (8, 20, 8, 6));
        }
        if matches!(stage, Stage::Scaffold | Stage::Roofed) && span >= 2.0 {
            let (p, inside) = place(span * 0.25, 0.42);
            push(Kind::Skip, p, 0.0, key_for(p, inside), (20, 16, 20, 8));
        }
        // Cranes lift the frame and the cladding; the tower crane stays (jib
        // down) until the finished building replaces the shell.
        if matches!(stage, Stage::Framing | Stage::Scaffold | Stage::Roofed) {
            if h >= 40 {
                let reach = (lw.max(lh) * 0.9 + 1.2).clamp(2.2, 4.0);
                let p = (lx + 0.4, ly + 0.4);
                let mast = h + 26;
                let ex = (reach * 46.0) as i32 + 12;
                let ey = (reach * 23.0) as i32 + 6;
                push(
                    Kind::Tower,
                    p,
                    reach,
                    base_key.saturating_sub(1),
                    (ex, mast + 16 + ey, ex, 8),
                );
                out.last_mut().unwrap().late = stage == Stage::Roofed;
            } else if h >= 24 && stage != Stage::Roofed {
                let p = at(span * 0.05, -0.3);
                let reach = 0.8;
                let ex = 90;
                push(Kind::Mobile, p, reach, base_key, (ex, h + 40, ex, 16));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Pose maths (pure functions of the work clock)
// ---------------------------------------------------------------------------

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub struct TowerPose {
    pub mast: i32,
    pub tip: (f32, f32),
    pub counter: (f32, f32),
    pub troll: (f32, f32),
    pub load_z: i32,
}

impl Rig {
    pub fn tower_pose(&self, clock: f32) -> TowerPose {
        let clock = if self.late { 0.0 } else { clock };
        let sf = sfv(self.seed);
        let a = 0.785 + 1.15 * (clock * 0.12 + sf).sin();
        let d = (a.cos(), a.sin());
        let mast = self.h + 26;
        let rt = self.reach * (0.52 + 0.36 * (clock * 0.19 + sf * 1.7).sin());
        let k = 0.5 + 0.5 * (clock * 0.27 + sf * 2.3).sin();
        TowerPose {
            mast,
            tip: (self.x + d.0 * self.reach, self.y + d.1 * self.reach),
            counter: (self.x - d.0 * self.reach * 0.38, self.y - d.1 * self.reach * 0.38),
            troll: (self.x + d.0 * rt, self.y + d.1 * rt),
            load_z: lerp((self.h + 10) as f32, (mast - 8) as f32, k) as i32,
        }
    }

    /// Mobile crane: boom tip (tile), tip height, cable length.
    pub fn mobile_pose(&self, clock: f32) -> ((f32, f32), i32, i32) {
        let sf = sfv(self.seed);
        let slew = 0.32 * (clock * 0.13 + sf).sin();
        let (c, s) = (slew.cos(), slew.sin());
        let d = (self.inward.0 * c - self.inward.1 * s, self.inward.0 * s + self.inward.1 * c);
        let reach = self.reach + 0.45 + 0.4 * (clock * 0.16 + sf * 1.3).sin();
        let tip = (self.x + d.0 * reach, self.y + d.1 * reach);
        let tz = (self.h + 12).max(34);
        let cable = 7 + (9.0 * (0.5 + 0.5 * (clock * 0.21 + sf).sin())) as i32;
        (tip, tz, cable)
    }

    /// Excavator arm angle (radians) and the swing towards the spoil heap.
    pub fn dig_angle(&self, clock: f32) -> f32 {
        let sf = sfv(self.seed);
        let t = (clock * 0.16 + sf).rem_euclid(2.0);
        let tri = if t < 1.0 { t } else { 2.0 - t };
        0.25 + 1.15 * tri
    }

    /// Truck visit cycle: (present, position along the axis in tiles, spin phase).
    pub fn visit(&self, clock: f32) -> (bool, f32, i32) {
        let period = 110.0;
        let t = (clock + sfv(self.seed) * 7.0).rem_euclid(period);
        let (arrive, park, leave) = (9.0, 62.0, 9.0);
        if t < arrive {
            (true, lerp(-2.0, 0.0, t / arrive), 0)
        } else if t < arrive + park {
            (true, 0.0, ((t - arrive) * 2.2) as i32)
        } else if t < arrive + park + leave {
            (true, lerp(0.0, 2.0, (t - arrive - park) / leave), 0)
        } else {
            (false, 0.0, 0)
        }
    }

    /// A number that changes exactly when the rig's pixels do.
    pub fn signature(&self, clock: f32, work: bool) -> u64 {
        let q = |v: f32| (v * 2.0).round() as i64 as u64;
        match self.kind {
            Kind::Tower if self.late => 7,
            Kind::Tower => {
                let p = self.tower_pose(clock);
                let mut h = 0u64;
                for v in [p.tip.0, p.tip.1, p.troll.0, p.troll.1, p.counter.0, p.counter.1] {
                    h = h.wrapping_mul(0x100_0000_01B3) ^ q(v * 16.0);
                }
                h.wrapping_mul(31) ^ p.load_z as u64
            }
            Kind::Mobile => {
                let (t, z, c) = self.mobile_pose(clock);
                let mut h = q(t.0 * 16.0);
                h = h.wrapping_mul(0x100_0000_01B3) ^ q(t.1 * 16.0);
                h.wrapping_mul(31) ^ (z as u64) ^ ((c as u64) << 20)
            }
            Kind::Excavator => (self.dig_angle(clock) * 40.0) as u64,
            Kind::Mixer | Kind::Truck => {
                let (here, pos, spin) = self.visit(clock);
                if !work && !here {
                    return 0;
                }
                (here as u64) | ((pos * 20.0) as i64 as u64) << 1 | ((spin & 1) as u64) << 40
            }
            _ => 1,
        }
    }

    /// The box the rig's moving parts cover right now (for damage).
    pub fn bbox(&self, cam: &Camera, clock: f32, sh: Option<&ActorShadow>) -> Rect {
        match self.kind {
            Kind::Tower => {
                let p = self.tower_pose(clock);
                let mut pts: Vec<(f32, f32)> = vec![
                    cam.project(self.x, self.y, self.z + p.mast + 14),
                    cam.project(p.tip.0, p.tip.1, self.z + p.mast + 2),
                    cam.project(p.counter.0, p.counter.1, self.z + p.mast - 6),
                    cam.project(p.troll.0, p.troll.1, self.z + p.load_z - 6),
                ];
                pts.push(cam.project(p.troll.0, p.troll.1, self.z + p.mast));
                if let Some(s) = sh {
                    let m = p.mast as f32;
                    for (q, z) in [
                        ((self.x, self.y), m),
                        (p.tip, m),
                        (p.counter, m),
                        (p.troll, p.load_z as f32),
                    ] {
                        pts.push(cam.project(q.0 + s.dx * z, q.1 + s.dy * z, self.z));
                    }
                }
                bbox_of(&pts, 4)
            }
            Kind::Mobile => {
                let (tip, tz, cable) = self.mobile_pose(clock);
                let pts = [
                    cam.project(self.x, self.y, self.z + 8),
                    cam.project(tip.0, tip.1, self.z + tz),
                    cam.project(tip.0, tip.1, self.z + tz - cable - 8),
                ];
                let mut pts = pts.to_vec();
                if let Some(s) = sh {
                    pts.push(cam.project(tip.0 + s.dx * tz as f32, tip.1 + s.dy * tz as f32, self.z));
                }
                bbox_of(&pts, 4)
            }
            _ => self.bounds,
        }
    }
}

fn bbox_of(pts: &[(f32, f32)], pad: i32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    Rect::new(x0 as i32 - pad, y0 as i32 - pad, x1 as i32 + pad + 1, y1 as i32 + pad + 1)
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// A depth-tested, tinted pen over the frame.
pub struct Pen<'a> {
    pub fb: &'a mut Framebuffer,
    pub depth: &'a [u16],
    pub key: u16,
    pub tint: [f32; 3],
    pub glow: f32,
}

impl<'a> Pen<'a> {
    fn tinted(&self, c: Color) -> Color {
        palette::rgb(
            (palette::r(c) as f32 * self.tint[0]).min(255.0) as u8,
            (palette::g(c) as f32 * self.tint[1]).min(255.0) as u8,
            (palette::b(c) as f32 * self.tint[2]).min(255.0) as u8,
        )
    }

    #[inline]
    fn put_c(&mut self, x: i32, y: i32, c: Color) {
        let clip = self.fb.clip();
        if x < clip.x0 || x >= clip.x1 || y < clip.y0 || y >= clip.y1 {
            return;
        }
        let i = self.fb.index(x, y);
        if !self.depth.is_empty() && self.depth[i] > self.key {
            return;
        }
        self.fb.px_mut()[i] = c;
    }

    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        let c = self.tinted(c);
        self.put_c(x, y, c);
    }

    /// A light: never tinted by the ambient.
    pub fn put_lit(&mut self, x: i32, y: i32, c: Color) {
        self.put_c(x, y, c);
    }

    pub fn glow_at(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        let clip = self.fb.clip();
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (px, py) = (x + dx, y + dy);
            if px >= clip.x0 && px < clip.x1 && py >= clip.y0 && py < clip.y1 {
                self.fb.put_blended(px, py, c, cov);
            }
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        let c = self.tinted(c);
        for yy in y..y + h {
            for xx in x..x + w {
                self.put_c(xx, yy, c);
            }
        }
    }

    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color) {
        let c = self.tinted(c);
        let (x0, y0) = (a.0.round() as i32, a.1.round() as i32);
        let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.put_c(x, y, c);
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

    pub fn poly(&mut self, pts: &[(f32, f32)], c: Color) {
        let c = self.tinted(c);
        let bb = bbox_of(pts, 0);
        let clip = self.fb.clip();
        let (y0, y1) = (bb.y0.max(clip.y0), bb.y1.min(clip.y1));
        let mut xs = [0f32; 12];
        for y in y0..y1 {
            let yc = y as f32 + 0.5;
            let mut n = 0;
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                if ((a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc)) && n < xs.len() {
                    xs[n] = a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0);
                    n += 1;
                }
            }
            xs[..n].sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < n {
                let xa = (xs[i] - 0.5).ceil() as i32;
                let xb = (xs[i + 1] - 0.5).ceil() as i32;
                for x in xa..xb {
                    self.put_c(x, y, c);
                }
                i += 2;
            }
        }
    }

    /// An axis-aligned iso box centred at `c` with half extents `h`, from
    /// height `z0` to `z1`; top, left (+y) and right (+x) faces.
    #[allow(clippy::too_many_arguments)]
    pub fn cube(
        &mut self,
        cam: &Camera,
        gz: i32,
        c: (f32, f32),
        h: (f32, f32),
        z0: i32,
        z1: i32,
        col: (Color, Color, Color),
    ) {
        let p = |x: f32, y: f32, z: i32| cam.project(x, y, gz + z);
        let (x0, y0, x1, y1) = (c.0 - h.0, c.1 - h.1, c.0 + h.0, c.1 + h.1);
        self.poly(&[p(x0, y1, z1), p(x1, y1, z1), p(x1, y1, z0), p(x0, y1, z0)], col.1);
        self.poly(&[p(x1, y0, z1), p(x1, y1, z1), p(x1, y1, z0), p(x1, y0, z0)], col.2);
        self.poly(&[p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)], col.0);
    }
}

fn faces(c: Color) -> (Color, Color, Color) {
    (palette::shade(c, 1.08), palette::shade(c, pal::FACE_LEFT), palette::shade(c, pal::FACE_RIGHT))
}

impl Rig {
    /// Half extents of a body `len` long (along the frontage axis) and `wid`
    /// across it.
    fn half(&self, len: f32, wid: f32) -> (f32, f32) {
        if self.axis.0.abs() > 0.5 { (len * 0.5, wid * 0.5) } else { (wid * 0.5, len * 0.5) }
    }

    /// Draw the rig into `pen`. `work` is false at night (crews gone).
    pub fn draw(&self, pen: &mut Pen, cam: &Camera, clock: f32, work: bool) {
        let night = pen.glow > 0.45;
        match self.kind {
            Kind::Tower => self.draw_tower(pen, cam, clock, night),
            Kind::Mobile => self.draw_mobile(pen, cam, clock),
            Kind::Excavator => self.draw_excavator(pen, cam, clock),
            Kind::Mixer | Kind::Truck => {
                let (here, pos, spin) = self.visit(clock);
                if here && work {
                    self.draw_truck(pen, cam, pos, spin);
                }
            }
            Kind::Cabin => self.draw_cabin(pen, cam, night),
            Kind::Toilet => {
                let c = (self.x, self.y);
                let h = (0.07, 0.07);
                let col = (
                    palette::shade(pal::CABIN, 1.0),
                    palette::shade(pal::TOILET, pal::FACE_LEFT + 0.1),
                    palette::shade(pal::TOILET, pal::FACE_RIGHT),
                );
                pen.cube(cam, self.z, c, h, 0, 13, col);
                // The door, lighter, on the near faces.
                let q = cam.project(c.0 - 0.02, c.1 + h.1, self.z);
                pen.rect(q.0 as i32 - 1, q.1 as i32 - 9, 3, 8, palette::shade(pal::TOILET, 1.3));
                pen.put(q.0 as i32 + 1, q.1 as i32 - 5, pal::CABIN);
            }
            Kind::Skip => {
                let c = (self.x, self.y);
                let h = self.half(0.5, 0.26);
                pen.cube(cam, self.z, c, h, 0, 8, faces(pal::SKIP));
                // Yellow hazard band and a heap of rubble over the rim.
                let p = |dx: f32, dy: f32, z: i32| cam.project(c.0 + dx, c.1 + dy, self.z + z);
                pen.line(p(-h.0, h.1, 7), p(h.0, h.1, 7), pal::SKIP_STRIPE);
                pen.line(p(h.0, -h.1, 7), p(h.0, h.1, 7), palette::shade(pal::SKIP_STRIPE, 0.8));
                for k in 0..7 {
                    let u = (k as f32 / 6.0 - 0.5) * 1.6;
                    let q = p(u * h.0, u * h.1 * 0.3, 9 + (k % 3) as i32);
                    let col = [pal::BRICK, pal::CONCRETE, pal::TIMBER, pal::DIRT[3]][k % 4];
                    pen.rect(q.0 as i32 - 1, q.1 as i32, 3, 2, col);
                }
            }
        }
    }

    fn draw_cabin(&self, pen: &mut Pen, cam: &Camera, night: bool) {
        let c = (self.x, self.y);
        let h = self.half(0.52, 0.3);
        pen.cube(cam, self.z, c, h, 0, 12, faces(pal::CABIN));
        // Roof overhang and a small unit on it.
        let p = |dx: f32, dy: f32, z: i32| cam.project(c.0 + dx, c.1 + dy, self.z + z);
        pen.line(p(-h.0, h.1, 12), p(h.0, h.1, 12), palette::shade(pal::CABIN, 0.7));
        pen.line(p(h.0, -h.1, 12), p(h.0, h.1, 12), palette::shade(pal::CABIN, 0.55));
        let q = p(h.0 * 0.4, -h.1 * 0.2, 13);
        pen.rect(q.0 as i32 - 1, q.1 as i32 - 2, 3, 2, pal::STEEL);
        // Two windows and a door on the big face.
        let (a, b) = if self.axis.0.abs() > 0.5 { ((-1.0, 0.0), (1.0, 0.0)) } else { ((0.0, -1.0), (0.0, 1.0)) };
        let _ = (a, b);
        let win = if night { pal::WINDOW_LIT } else { pal::WINDOW_DARK };
        for f in [-0.55f32, 0.05] {
            let q = if self.axis.0.abs() > 0.5 {
                p(f * h.0, h.1, 8)
            } else {
                p(h.0, f * h.1, 8)
            };
            if night {
                pen.put_lit(q.0 as i32, q.1 as i32, win);
                pen.put_lit(q.0 as i32 + 1, q.1 as i32, win);
                pen.put_lit(q.0 as i32, q.1 as i32 + 1, win);
                pen.put_lit(q.0 as i32 + 1, q.1 as i32 + 1, win);
            } else {
                pen.rect(q.0 as i32, q.1 as i32, 2, 2, win);
            }
        }
        let q = if self.axis.0.abs() > 0.5 { p(0.62 * h.0, h.1, 0) } else { p(h.0, 0.62 * h.1, 0) };
        pen.rect(q.0 as i32, q.1 as i32 - 7, 2, 7, palette::shade(pal::DOOR, 1.0));
    }

    fn draw_truck(&self, pen: &mut Pen, cam: &Camera, pos: f32, spin: i32) {
        let c0 = (self.x + self.axis.0 * pos, self.y + self.axis.1 * pos);
        let gz = self.z;
        let along = |a: f32| (c0.0 + self.axis.0 * a, c0.1 + self.axis.1 * a);
        // Parts back to front along the axis: the cab at the near end.
        let flip = if self.seed % 2 == 0 { 1.0 } else { -1.0 };
        let _ = flip;
        let chassis = self.half(0.78, 0.22);
        pen.cube(cam, gz, along(0.0), chassis, 2, 5, faces(pal::TRACK));
        // Wheels: dark blocks at the front and rear axles.
        for a in [-0.27f32, 0.0, 0.27] {
            let q = cam.project(along(a).0 + self.axis.1 * 0.12, along(a).1 + self.axis.0 * 0.12, gz);
            pen.rect(q.0 as i32 - 2, q.1 as i32 - 3, 4, 3, pal::TRACK);
        }
        let cab_pal = if self.kind == Kind::Mixer { pal::DRUM_STRIPE } else { pal::CABIN };
        pen.cube(cam, gz, along(0.32), self.half(0.2, 0.22), 5, 14, faces(cab_pal));
        let q = cam.project(along(0.32).0, along(0.32).1, gz + 12);
        pen.put(q.0 as i32 - 1, q.1 as i32, pal::WINDOW_DARK);
        pen.put(q.0 as i32, q.1 as i32, pal::WINDOW_DARK);
        if self.kind == Kind::Mixer {
            // The drum: a long pale barrel with orange stripes that turn.
            let d = self.half(0.5, 0.21);
            let c = along(-0.1);
            pen.cube(cam, gz, c, d, 5, 16, faces(pal::DRUM));
            let n = 5;
            for i in 0..n {
                if (i as i32 + spin) % 2 == 0 {
                    let (a0, a1) = (-0.25 + 0.5 * i as f32 / n as f32, -0.25 + 0.5 * (i + 1) as f32 / n as f32);
                    let p = |a: f32, z: i32| {
                        let pt = along(-0.1 + a);
                        if self.axis.0.abs() > 0.5 {
                            cam.project(pt.0, pt.1 + d.1, gz + z)
                        } else {
                            cam.project(pt.0 + d.0, pt.1, gz + z)
                        }
                    };
                    let (lum_c) = if self.axis.0.abs() > 0.5 { pal::FACE_LEFT } else { pal::FACE_RIGHT };
                    pen.poly(&[p(a0, 16), p(a1, 16), p(a1, 5), p(a0, 5)], palette::shade(pal::DRUM_STRIPE, lum_c));
                }
            }
            // The hopper on top at the rear.
            let q = cam.project(along(-0.38).0, along(-0.38).1, gz + 17);
            pen.rect(q.0 as i32 - 1, q.1 as i32 - 2, 3, 3, pal::STEEL);
        } else {
            // Flatbed with a pallet of bricks and a stack of plasterboard.
            let bed = self.half(0.56, 0.22);
            pen.cube(cam, gz, along(-0.1), bed, 5, 7, faces(pal::CONCRETE));
            pen.cube(cam, gz, along(-0.22), self.half(0.2, 0.15), 7, 12, faces(pal::BRICK));
            pen.cube(cam, gz, along(0.0), self.half(0.18, 0.15), 7, 10, faces(pal::SHEATHING));
        }
    }

    fn draw_excavator(&self, pen: &mut Pen, cam: &Camera, clock: f32) {
        let gz = self.z;
        let c = (self.x, self.y);
        let d = self.inward;
        // Tracks, body, cab.
        pen.cube(cam, gz, c, self.half(0.36, 0.2), 0, 3, faces(pal::TRACK));
        pen.cube(cam, gz, c, self.half(0.26, 0.17), 3, 8, faces(pal::CRANE));
        let cab = (c.0 - d.0 * 0.05, c.1 - d.1 * 0.05);
        pen.cube(cam, gz, (cab.0 - d.0 * 0.04, cab.1 - d.1 * 0.04), (0.07, 0.07), 8, 14, faces(pal::CRANE));
        let w = cam.project(cab.0 + d.0 * 0.04, cab.1 + d.1 * 0.04 + 0.0, gz + 12);
        pen.put(w.0 as i32, w.1 as i32, pal::WINDOW_DARK);
        pen.put(w.0 as i32 + 1, w.1 as i32, pal::WINDOW_DARK);
        // Counterweight at the back.
        pen.cube(cam, gz, (c.0 - d.0 * 0.12, c.1 - d.1 * 0.12), (0.05, 0.05), 3, 9, faces(pal::COUNTERWEIGHT));
        // The arm: boom up and out, stick swinging down into the pit.
        let phi = self.dig_angle(clock);
        let at = |s: f32, z: f32| {
            let q = cam.project(c.0 + d.0 * s, c.1 + d.1 * s, gz + z as i32);
            (q.0, q.1)
        };
        let pivot = at(0.08, 9.0);
        let elbow = at(0.52, 20.0);
        let wrist = at(0.52 + 0.46 * phi.cos(), 20.0 - 30.0 * phi.sin());
        for off in [0.0f32, 1.0] {
            pen.line((pivot.0, pivot.1 - off), (elbow.0, elbow.1 - off), if off == 0.0 { pal::CRANE } else { pal::CRANE_D });
            pen.line((elbow.0, elbow.1 - off), (wrist.0, wrist.1 - off), if off == 0.0 { pal::CRANE } else { pal::CRANE_D });
        }
        // The bucket, with earth in it when it is low.
        let (bx, by) = (wrist.0.round() as i32, wrist.1.round() as i32);
        pen.rect(bx - 1, by, 3, 3, pal::STEEL_D);
        if phi > 0.7 {
            pen.rect(bx - 1, by - 1, 3, 1, pal::DIRT[2]);
        }
    }

    fn draw_tower(&self, pen: &mut Pen, cam: &Camera, clock: f32, night: bool) {
        let p = self.tower_pose(clock);
        let s = cam.project(self.x, self.y, self.z);
        let (sx, sy) = (s.0.round() as i32, s.1.round() as i32);
        let mast = p.mast;
        // The mast: two legs and a zig-zag of bracing, behind the building.
        for k in 0..=mast {
            pen.put(sx - 1, sy - k, pal::CRANE);
            pen.put(sx + 1, sy - k, pal::CRANE_D);
        }
        let mut z = 0;
        while z + 5 <= mast {
            let (a, b) = if (z / 5) % 2 == 0 { (-1, 1) } else { (1, -1) };
            pen.put(sx, sy - z - 2, pal::CRANE_D);
            pen.put(sx + a, sy - z, pal::CRANE_D);
            pen.put(sx + b, sy - z - 5, pal::CRANE_D);
            z += 5;
        }
        // Everything above the building is in the open air: no depth test.
        pen.key = u16::MAX;
        let top = |t: (f32, f32), z: i32| cam.project(t.0, t.1, self.z + z);
        let hub_top = top((self.x, self.y), mast + 3);
        let hub_bot = top((self.x, self.y), mast - 3);
        // Tower head (the A-frame) and the cab.
        let apex = (hub_top.0, hub_top.1 - 11.0);
        pen.line((hub_top.0 - 1.0, hub_top.1), apex, pal::CRANE);
        pen.line((hub_top.0 + 1.0, hub_top.1), apex, pal::CRANE_D);
        pen.rect(sx + 1, sy - mast - 2, 4, 4, pal::CABIN);
        pen.put(sx + 4, sy - mast - 1, pal::WINDOW_DARK);
        // Jib: two chords and a lattice of webs.
        let draw_arm = |pen: &mut Pen, tip: (f32, f32)| {
            let a_top = top((self.x, self.y), mast + 2);
            let a_bot = top((self.x, self.y), mast - 2);
            let b_top = top(tip, mast + 2);
            let b_bot = top(tip, mast - 2);
            pen.line(a_top, b_top, pal::CRANE);
            pen.line(a_bot, b_bot, pal::CRANE_D);
            let n = (((b_top.0 - a_top.0).abs() + (b_top.1 - a_top.1).abs()) / 5.0).max(2.0) as i32;
            for i in 0..n {
                let (t0, t1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
                let (u, v) = if i % 2 == 0 { (t0, t1) } else { (t1, t0) };
                let from = (a_top.0 + (b_top.0 - a_top.0) * u, a_top.1 + (b_top.1 - a_top.1) * u);
                let to = (a_bot.0 + (b_bot.0 - a_bot.0) * v, a_bot.1 + (b_bot.1 - a_bot.1) * v);
                pen.line(from, to, pal::CRANE_D);
            }
        };
        if !self.late {
            draw_arm(pen, p.tip);
        }
        draw_arm(pen, p.counter);
        // Pendants from the apex.
        let tip = top(p.tip, mast + 2);
        let ctr = top(p.counter, mast + 2);
        if !self.late {
            pen.line(apex, tip, pal::STEEL_D);
        }
        pen.line(apex, ctr, pal::STEEL_D);
        // Counterweight blocks.
        let cw = top(p.counter, mast - 3);
        pen.rect(cw.0 as i32 - 2, cw.1 as i32, 5, 5, pal::COUNTERWEIGHT);
        pen.rect(cw.0 as i32 - 2, cw.1 as i32, 5, 1, palette::shade(pal::COUNTERWEIGHT, 1.2));
        let (t, l, lx, ly) = {
            let t = top(p.troll, mast - 2);
            let l = top(p.troll, p.load_z);
            (t, l, l.0.round() as i32, l.1.round() as i32)
        };
        if !self.late {
        // Trolley, hook cable and the load.
        pen.rect(t.0 as i32 - 1, t.1 as i32, 3, 2, pal::STEEL_D);
        pen.line((t.0, t.1 + 2.0), (l.0, l.1 - 3.0), pal::STEEL_D);
        match self.seed % 3 {
            0 => {
                pen.rect(lx - 3, ly - 3, 7, 4, pal::PLANK);
                pen.rect(lx - 3, ly - 3, 7, 1, palette::shade(pal::PLANK, 1.2));
            }
            1 => {
                pen.rect(lx - 3, ly - 3, 7, 4, pal::STEEL);
                pen.rect(lx - 3, ly - 1, 7, 1, pal::STEEL_D);
            }
            _ => {
                pen.rect(lx - 2, ly - 4, 5, 5, pal::CONCRETE);
                pen.rect(lx - 1, ly - 5, 3, 1, pal::STEEL_D);
            }
        }
        pen.line((l.0 - 3.0, l.1 - 3.0), (l.0, l.1 - 5.0), pal::STEEL_D);
        pen.line((l.0 + 3.0, l.1 - 3.0), (l.0, l.1 - 5.0), pal::STEEL_D);
        }
        let _ = hub_bot;
        // The aviation light on the apex: steady red, brighter at night, with
        // a pool of work light under the jib once it is dark.
        let (ax, ay) = (apex.0.round() as i32, apex.1.round() as i32);
        pen.put_lit(ax, ay - 1, pal::CRANE_LIGHT);
        pen.put_lit(ax, ay, pal::CRANE_LIGHT);
        if night {
            pen.put_lit(ax - 1, ay - 1, pal::CRANE_LIGHT);
            pen.put_lit(ax + 1, ay - 1, pal::CRANE_LIGHT);
            pen.put_lit(ax, ay - 2, pal::CRANE_LIGHT);
            pen.glow_at(ax, ay - 1, pal::CRANE_LIGHT, 0.7);
            for (dx, dy) in [(2, -1), (-2, -1), (0, -3), (0, 1), (1, -2), (-1, -2)] {
                pen.glow_at(ax + dx, ay + dy, pal::CRANE_LIGHT, 0.3);
            }
            // The work lamp under the jib: a warm pool on the load.
            let (tx, ty) = (t.0.round() as i32, t.1.round() as i32);
            pen.put_lit(tx, ty + 2, pal::WINDOW_LIT);
            pen.glow_at(tx, ty + 3, pal::WINDOW_LIT, 0.35);
            pen.glow_at(lx, ly - 2, pal::WINDOW_LIT, 0.3);
        }
    }

    fn draw_mobile(&self, pen: &mut Pen, cam: &Camera, clock: f32) {
        let gz = self.z;
        let c = (self.x, self.y);
        let at = |a: f32| (c.0 + self.axis.0 * a, c.1 + self.axis.1 * a);
        // Outrigger pads and arms.
        for (a, o) in [(-0.3f32, 0.2f32), (0.3, 0.2), (-0.3, -0.2), (0.3, -0.2)] {
            let pt = (at(a).0 + self.axis.1 * o, at(a).1 + self.axis.0 * o);
            let q = cam.project(pt.0, pt.1, gz);
            pen.rect(q.0 as i32 - 1, q.1 as i32 - 1, 3, 2, pal::STEEL_D);
            let b = cam.project(at(a).0, at(a).1, gz + 3);
            pen.line((b.0, b.1), (q.0, q.1 - 1.0), pal::STEEL);
        }
        pen.cube(cam, gz, at(0.0), self.half(0.8, 0.26), 2, 7, faces(pal::CRANE_RED));
        pen.cube(cam, gz, at(0.32), self.half(0.2, 0.24), 7, 15, faces(pal::CRANE_RED));
        let q = cam.project(at(0.32).0, at(0.32).1, gz + 13);
        pen.put(q.0 as i32, q.1 as i32, pal::WINDOW_DARK);
        pen.put(q.0 as i32 + 1, q.1 as i32, pal::WINDOW_DARK);
        for a in [-0.28f32, 0.0, 0.28] {
            let wq = cam.project(at(a).0 + self.axis.1 * 0.13, at(a).1 + self.axis.0 * 0.13, gz);
            pen.rect(wq.0 as i32 - 2, wq.1 as i32 - 3, 4, 3, pal::TRACK);
        }
        pen.cube(cam, gz, at(-0.1), (0.1, 0.1), 7, 11, faces(pal::CRANE_D));
        // The boom runs from the turntable over the building; the tip and the
        // hook are in the open air, so no depth test beyond the truck.
        let (tip, tz, cable) = self.mobile_pose(clock);
        let pivot = cam.project(at(-0.1).0, at(-0.1).1, gz + 11);
        let tp = cam.project(tip.0, tip.1, gz + tz);
        pen.key = u16::MAX;
        for off in [0.0f32, 1.0, 2.0] {
            let col = if off == 1.0 { pal::CRANE } else { pal::CRANE_D };
            pen.line((pivot.0, pivot.1 - off), (tp.0, tp.1 - off), col);
        }
        // The telescoped tip is lighter, the pulley a dark dot.
        let mid = ((pivot.0 + tp.0 * 2.0) / 3.0, (pivot.1 + tp.1 * 2.0) / 3.0);
        pen.line((mid.0, mid.1 - 1.0), (tp.0, tp.1 - 1.0), palette::shade(pal::CRANE, 1.2));
        pen.rect(tp.0 as i32 - 1, tp.1 as i32 - 1, 3, 3, pal::STEEL_D);
        let hook = (tp.0, tp.1 + cable as f32);
        pen.line((tp.0, tp.1 + 2.0), hook, pal::STEEL_D);
        pen.rect(hook.0 as i32 - 2, hook.1 as i32, 5, 3, pal::PLANK);
        pen.rect(hook.0 as i32 - 2, hook.1 as i32, 5, 1, palette::shade(pal::PLANK, 1.2));
    }

    /// The shadows this rig throws, from the same light as everything else.
    pub fn shadow(&self, dst: &mut Framebuffer, depth: &[u16], cam: &Camera, s: &ActorShadow, clock: f32) {
        let key = self.key;
        let gz = self.z;
        let ground = |x: f32, y: f32| cam.project(x, y, gz);
        let mut line = |dst: &mut Framebuffer, a: (f32, f32), b: (f32, f32)| {
            shadow_line(dst, depth, key, a, b, s);
        };
        match self.kind {
            Kind::Tower => {
                let p = self.tower_pose(clock);
                let m = p.mast as f32;
                let off = |x: f32, y: f32, z: f32| ground(x + s.dx * z, y + s.dy * z);
                line(dst, ground(self.x, self.y), off(self.x, self.y, m));
                let a = off(self.x, self.y, m);
                let tip = off(p.tip.0, p.tip.1, m);
                let ctr = off(p.counter.0, p.counter.1, m);
                if !self.late {
                    line(dst, a, tip);
                }
                line(dst, a, ctr);
                // The load, hanging from the trolley.
                if !self.late {
                    let t = off(p.troll.0, p.troll.1, m);
                    let l = off(p.troll.0, p.troll.1, p.load_z as f32);
                    line(dst, t, l);
                }
            }
            Kind::Mobile => {
                let (tip, tz, _) = self.mobile_pose(clock);
                let off = |x: f32, y: f32, z: f32| ground(x + s.dx * z, y + s.dy * z);
                let base = (self.x - self.axis.0 * 0.1, self.y - self.axis.1 * 0.1);
                line(dst, off(base.0, base.1, 11.0), off(tip.0, tip.1, tz as f32));
            }
            _ => {
                let (hw, hh, ht) = match self.kind {
                    Kind::Excavator => (0.16, 0.12, 14.0),
                    Kind::Mixer | Kind::Truck => {
                        let (here, pos, _) = self.visit(clock);
                        if !here {
                            return;
                        }
                        let _ = pos;
                        (0.3, 0.14, 16.0)
                    }
                    Kind::Cabin => (0.28, 0.16, 12.0),
                    Kind::Toilet => (0.07, 0.07, 13.0),
                    _ => (0.25, 0.14, 8.0),
                };
                let (x, y) = if matches!(self.kind, Kind::Mixer | Kind::Truck) {
                    let (_, pos, _) = self.visit(clock);
                    (self.x + self.axis.0 * pos, self.y + self.axis.1 * pos)
                } else {
                    (self.x, self.y)
                };
                let _ = crate::light::draw_actor_shadow(dst, cam, depth, key, x, y, gz, hw, hh, ht, s, true);
            }
        }
    }
}

/// A one-pixel blended line on the ground, depth-tested like every other
/// shadow (a wall in front of it hides it).
fn shadow_line(fb: &mut Framebuffer, depth: &[u16], key: u16, a: (f32, f32), b: (f32, f32), s: &ActorShadow) {
    // B68: the multiply of a shaded surface, not a grey paint; a one-pixel
    // boom is thinner than a body, so it is a little lighter.
    let cov = s.cov * 0.8;
    let f = [1.0 + (s.mult[0] - 1.0) * cov, 1.0 + (s.mult[1] - 1.0) * cov, 1.0 + (s.mult[2] - 1.0) * cov];
    let (x0, y0) = (a.0.round() as i32, a.1.round() as i32);
    let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    let clip = fb.clip();
    loop {
        if x >= clip.x0 && x < clip.x1 && y >= clip.y0 && y < clip.y1 {
            let i = fb.index(x, y);
            let d = if depth.is_empty() { 0 } else { depth[i] };
            if !(d != 0 && d & 1 == 0 && d > key) {
                let c = fb.px()[i];
                if c != palette::TRANSPARENT {
                    fb.px_mut()[i] = palette::with_alpha(
                        palette::rgb(
                            (palette::r(c) as f32 * f[0]) as u8,
                            (palette::g(c) as f32 * f[1]) as u8,
                            (palette::b(c) as f32 * f[2]) as u8,
                        ),
                        palette::alpha(c),
                    );
                }
            }
        }
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

#[allow(dead_code)]
fn _unused(_: ObjKind) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> map::TileMap {
        map::generate_with(3, 64, 64, None).unwrap()
    }

    fn lot(kind: ObjKind, storeys: u8, stage: Stage) -> CityObj {
        CityObj {
            x: 20,
            y: 20,
            w: 2,
            h: 2,
            level: 3,
            kind,
            storeys,
            style: 0,
            stage,
            seed: 5,
            front: 1,
            t: 0.0,
            kit: 0, wear: Default::default(),
        }
    }

    #[test]
    fn a_tall_frame_gets_a_tower_crane_and_a_finished_lot_none() {
        let cam = Camera::new(1000, 100);
        let r = rigs_for(&[lot(ObjKind::Tower, 20, Stage::Framing)], &cam, &flat());
        assert!(r.iter().any(|g| g.kind == Kind::Tower));
        assert!(r.iter().any(|g| g.kind == Kind::Cabin));
        assert!(rigs_for(&[lot(ObjKind::Tower, 20, Stage::Finished)], &cam, &flat()).is_empty());
        let r = rigs_for(&[lot(ObjKind::House, 2, Stage::Graded)], &cam, &flat());
        assert!(r.iter().any(|g| g.kind == Kind::Excavator));
        assert!(!r.iter().any(|g| g.kind == Kind::Tower));
    }

    #[test]
    fn the_crane_swings_and_a_stopped_clock_changes_nothing() {
        let cam = Camera::new(1000, 100);
        let r = rigs_for(&[lot(ObjKind::Tower, 20, Stage::Framing)], &cam, &flat());
        let c = r.iter().find(|g| g.kind == Kind::Tower).unwrap();
        assert_ne!(c.signature(0.0, true), c.signature(30.0, true));
        assert_eq!(c.signature(12.0, false), c.signature(12.0, false));
    }

    #[test]
    fn hard_hats_are_painted_on_hi_vis_workers() {
        let r = crate::people::PeopleRenderer::new();
        let (w, h, px) = r.sheet(16);
        let hats = px.iter().filter(|&&c| c == pal::HARDHAT || c == pal::HARDHAT_W).count();
        assert!(hats > 20, "{hats} hard-hat pixels in {w}x{h}");
    }
}
