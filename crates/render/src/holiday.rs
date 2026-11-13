//! Holidays as the renderer needs them (B36, D11/D60): a small look struct the
//! caller fills from `world::calendar`, plus the overlay drawing that dresses a
//! city for the day.
//!
//! `render` does not depend on `world`, so the holiday arrives as data: which
//! holiday it is, the garland palette from the ported `HOLIDECOR` table, and the
//! window tint it asks for. Everything here is deterministic from the look and
//! the tile, so a dirty-rect repaint draws the same decoration.

use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, level_px};
use crate::palette::{self, Color, pal};
use crate::tex;

/// Which holiday the city is dressed for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Holiday {
    #[default]
    None,
    Xmas,
    Halloween,
    July4,
    Nye,
    Valentine,
    StPatricks,
    Thanks,
    Easter,
    /// B53: the Bills game is on (pre-game or live): blue/red flags, a banner
    /// on the square, tailgaters and grills at the stadium lot.
    Bills,
    /// B53: a Bills win after dark: the Bills dressing plus fireworks.
    BillsWin,
    /// B53 personal date: balloons on the houses.
    Balloons,
    /// B53 personal date: a banner on the square.
    Banner,
    /// B53 personal date: fireworks in the evening.
    Fireworks,
    /// B53 personal date: flowers on the doorsteps.
    Flowers,
}

impl Holiday {
    /// The prop that hangs on a house (the JS `prop` field).
    pub fn prop(self) -> Prop {
        match self {
            Holiday::Xmas => Prop::Wreath,
            Holiday::Halloween => Prop::Pumpkin,
            Holiday::July4 => Prop::Flag,
            Holiday::Nye => Prop::Balloon,
            Holiday::Valentine => Prop::Heart,
            Holiday::StPatricks => Prop::Clover,
            Holiday::Thanks => Prop::Leaf,
            Holiday::Easter => Prop::Egg,
            Holiday::Bills | Holiday::BillsWin => Prop::Flag,
            Holiday::Balloons => Prop::Balloon,
            Holiday::Flowers => Prop::Flower,
            Holiday::Banner | Holiday::Fireworks | Holiday::None => Prop::None,
        }
    }

    pub fn is_night_fireworks(self) -> bool {
        matches!(
            self,
            Holiday::Nye | Holiday::July4 | Holiday::BillsWin | Holiday::Fireworks
        )
    }

    /// The square gets a banner (the Bills, or a personal banner date).
    pub fn has_banner(self) -> bool {
        matches!(self, Holiday::Bills | Holiday::BillsWin | Holiday::Banner | Holiday::Balloons)
    }

    /// The stadium lot fills with tailgaters.
    pub fn is_bills(self) -> bool {
        matches!(self, Holiday::Bills | Holiday::BillsWin)
    }
}

/// The decoration a house carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prop {
    None,
    Wreath,
    Pumpkin,
    Flag,
    Balloon,
    Heart,
    Clover,
    Leaf,
    Egg,
    Flower,
}

/// The whole holiday look: which day it is, its garland colours, and the window
/// tint the JS table asks for.
#[derive(Clone, Copy, Debug)]
pub struct HolidayLook {
    pub holiday: Holiday,
    /// Bulb colours, up to four (the `garland` rows of `HOLIDECOR`).
    pub garland: [Color; 4],
    /// How many of the four are used.
    pub bulbs: usize,
    /// Extra window warmth (0..1) the JS table applies.
    pub tint: f32,
    /// B76: how much of the town has its holiday lights up, 0..1. Residents put
    /// them up house by house through December and take them down in the first
    /// days of January; the switch here is the share of houses lit.
    pub lights: f32,
}

impl Default for HolidayLook {
    fn default() -> Self {
        Self {
            holiday: Holiday::None,
            garland: [pal::WINDOW_LIT; 4],
            bulbs: 0,
            tint: 0.0,
            lights: 0.0,
        }
    }
}

impl HolidayLook {
    pub fn new(holiday: Holiday, garland: &[[u8; 3]], tint: f32) -> Self {
        let mut g = [pal::WINDOW_LIT; 4];
        for (i, c) in garland.iter().take(4).enumerate() {
            g[i] = palette::rgb(c[0], c[1], c[2]);
        }
        Self {
            holiday,
            garland: g,
            bulbs: garland.len().min(4),
            tint,
            lights: 0.0,
        }
    }

    pub fn is_decorated(self) -> bool {
        self.holiday != Holiday::None && self.bulbs > 0
    }

    /// B76: are the town's own holiday lights up (a share of the houses)?
    pub fn is_lit(self) -> bool {
        self.lights > 0.02
    }

    #[inline]
    pub fn bulb(self, i: usize) -> Color {
        self.garland[i % self.bulbs.max(1)]
    }
}

/// A string of bulbs along a screen-space line: the eave of a house, a tree's
/// branch, a lamp post. Two pixels on, one off, so it reads as a string of
/// lights rather than a painted stripe.
pub fn garland_line(
    fb: &mut Framebuffer,
    a: (f32, f32),
    b: (f32, f32),
    look: &HolidayLook,
    salt: u32,
) {
    if !look.is_decorated() {
        return;
    }
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let steps = len.round() as i32;
    let mut k = 0usize;
    for i in 0..=steps {
        let t = i as f32 / steps.max(1) as f32;
        // Every third pixel is a bulb; the gap keeps it from reading as a bar.
        if i % 3 == 2 {
            continue;
        }
        let x = (a.0 + dx * t).round() as i32;
        let y = (a.1 + dy * t).round() as i32;
        let c = look.bulb(k);
        k += 1;
        fb.put(x, y, c);
        // A faint glow under each bulb so the string reads at night.
        fb.put_blended(x, y + 1, c, 0.35);
        let _ = salt;
    }
}

/// A house's decoration: a wreath on the door, a pumpkin at the step, a flag on
/// the wall. Drawn at the lot's street-facing corner, so it works for the kit
/// sprites too (they are blitted at the same anchor).
pub fn house_prop(
    fb: &mut Framebuffer,
    cam: &Camera,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    level: u8,
    front: u8,
    look: &HolidayLook,
    wall_h: i32,
) {
    if !look.is_decorated() {
        return;
    }
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (fx, fy) = match front {
        0 => ((x + w) as f32, y as f32 + h as f32 * 0.5),
        1 => (x as f32 + w as f32 * 0.5, (y + h) as f32),
        2 => (x as f32, y as f32 + h as f32 * 0.5),
        _ => (x as f32 + w as f32 * 0.5, y as f32),
    };
    // A string of lights along the street-facing eave, which is what makes a
    // decorated house read at night (B36).
    let eave = (w as f32 * 0.55).max(0.5) * 10.0;
    let (ex0, ey0, ex1, ey1) = match front {
        0 => ((x + w) as f32, y as f32 + 0.15, (x + w) as f32, (y + h) as f32 - 0.15),
        1 => (x as f32 + 0.15, (y + h) as f32, (x + w) as f32 - 0.15, (y + h) as f32),
        2 => (x as f32, y as f32 + 0.15, x as f32, (y + h) as f32 - 0.15),
        _ => (x as f32 + 0.15, y as f32, (x + w) as f32 - 0.15, y as f32),
    };
    let a = cam.project(ex0, ey0, base + (eave * 0.5) as i32);
    let b = cam.project(ex1, ey1, base + (eave * 0.5) as i32);
    // the owner 2026-11-10: the eave strings read as scratches across roofs at 1x; off.
    let _ = (a, b);

    let p = cam.project(fx, fy, base + 3);
    let (px, py) = (p.0 as i32, p.1 as i32);
    match look.holiday.prop() {
        Prop::Wreath => {
            // A lit green ring with a red bow, on the door.
            tex::ellipse(fb, p.0, p.1 - 3.0, 2.5, 2.0, palette::rgb(0x3f, 0xa0, 0x4a));
            fb.put(px, py - 3, palette::rgb(0x1d, 0x6a, 0x2e));
            fb.put(px, py - 4, pal::WINDOW_LIT);
            fb.put(px + 2, py - 3, palette::rgb(0xf0, 0x30, 0x40));
            fb.put(px - 2, py - 2, look.bulb(0));
        }
        Prop::Pumpkin => {
            // An orange pumpkin on the step, with a lit face at night.
            tex::ellipse(fb, p.0, p.1 - 1.0, 2.0, 1.5, palette::rgb(0xe8, 0x7a, 0x1a));
            fb.put(px, py - 3, palette::rgb(0x4a, 0x5a, 0x24));
            fb.put(px - 1, py - 1, pal::WINDOW_LIT);
            fb.put(px + 1, py - 1, pal::WINDOW_LIT);
        }
        Prop::Flag if look.holiday.is_bills() => {
            // B53/B62: a flag that reads at 1x. The pole stands on the roof's
            // front corner so the cloth hangs against the sky and the lawn, not
            // against a wall, and the cloth is two bold bands, 15 x 10 px, with
            // a swallow-tail. Generic blue and red; no club marks.
            let top = cam.project(fx, fy, base + wall_h);
            let (tx, ty) = (top.0 as i32, top.1 as i32);
            let pole = palette::rgb(0xe8, 0xe4, 0xd8);
            fb.line((top.0, top.1), (top.0, top.1 - 22.0), pole);
            let blue = palette::rgb(0x1f, 0x4c, 0xc0);
            let red = palette::rgb(0xd0, 0x22, 0x30);
            for i in 0..15i32 {
                // The cloth ripples: each column rides a pixel up or down.
                let wave = ((i as f32 * 0.55).sin() * 1.2).round() as i32;
                let h = if i > 12 { 10 - (i - 12) * 2 } else { 10 };
                for r in 0..h {
                    let c = if r == 0 || r == h - 1 {
                        palette::rgb(0xf4, 0xf4, 0xf4)
                    } else if r < 5 {
                        blue
                    } else {
                        red
                    };
                    fb.put(tx + 1 + i, ty - 22 + r + wave, c);
                }
            }
            fb.put(tx, ty - 23, palette::rgb(0xf0, 0xd0, 0x40));
        }
        Prop::Flag => {
            // A small flag: two stripes and a pole.
            fb.line((p.0, p.1), (p.0, p.1 - 5.0), palette::rgb(0xd8, 0xd4, 0xc8));
            for i in 0..4 {
                fb.put(px + 1 + i, py - 5, palette::rgb(0xd8, 0x28, 0x38));
                fb.put(px + 1 + i, py - 4, palette::rgb(0xf0, 0xf0, 0xf4));
                fb.put(px + 1 + i, py - 3, palette::rgb(0x3a, 0x5a, 0xc8));
            }
        }
        Prop::Balloon => {
            // B62: a bunch of three balloons tied to the porch rail: 7 px round,
            // a bright highlight, strings gathered at one knot. Readable at 1x.
            let (kx, ky) = (p.0, p.1 - 4.0);
            for (i, (dx, dy)) in [(-5.0f32, -15.0f32), (1.0, -19.0), (6.0, -14.0)].iter().enumerate() {
                let c = look.bulb(i);
                let (bx, by) = (kx + dx, ky + dy);
                fb.line((kx, ky), (bx, by + 4.0), palette::rgb(0xe8, 0xe4, 0xd8));
                tex::ellipse(fb, bx, by, 3.4, 4.2, c);
                fb.put(bx as i32 - 1, by as i32 - 2, palette::mix(c, palette::rgb(0xff, 0xff, 0xff), 0.7));
                fb.put(bx as i32, by as i32 + 4, palette::shade(c, 0.75));
            }
            fb.put(kx as i32, ky as i32, palette::rgb(0xe8, 0xe4, 0xd8));
        }
        Prop::Heart => {
            fb.put(px, py - 3, palette::rgb(0xe0, 0x30, 0x60));
            fb.put(px - 1, py - 4, palette::rgb(0xe0, 0x30, 0x60));
            fb.put(px + 1, py - 4, palette::rgb(0xe0, 0x30, 0x60));
            fb.put(px, py - 4, palette::rgb(0xff, 0x8a, 0xb0));
        }
        Prop::Clover => {
            fb.put(px, py - 3, palette::rgb(0x2f, 0xbf, 0x5a));
            fb.put(px - 1, py - 3, palette::rgb(0x2f, 0xbf, 0x5a));
            fb.put(px + 1, py - 3, palette::rgb(0x2f, 0xbf, 0x5a));
            fb.put(px, py - 4, palette::rgb(0x8f, 0xe0, 0xa0));
        }
        Prop::Leaf => {
            fb.put(px, py - 3, palette::rgb(0xd9, 0x82, 0x2b));
            fb.put(px + 1, py - 3, palette::rgb(0xb5, 0x65, 0x1d));
            fb.put(px, py - 4, palette::rgb(0xe0, 0xb0, 0x40));
        }
        Prop::Egg => {
            tex::ellipse(fb, p.0, p.1 - 2.0, 1.5, 2.0, look.bulb(1));
            fb.put(px, py - 3, palette::rgb(0xff, 0xff, 0xff));
        }
        Prop::Flower => {
            // A little pot of bright blooms on the step.
            fb.put(px, py - 2, palette::rgb(0x2e, 0x7a, 0x3e));
            fb.put(px - 1, py - 3, look.bulb(0));
            fb.put(px + 1, py - 3, look.bulb(1 % look.bulbs.max(1)));
            fb.put(px, py - 4, look.bulb(2 % look.bulbs.max(1)));
        }
        Prop::None => {}
    }
}

/// The plaza's big tree (B36): a cone of evergreen with a spiral of bulbs and a
/// star on top. Only at Christmas.
pub fn plaza_tree(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, look: &HolidayLook) {
    // B76: the town tree is lit for the whole holiday season, not one day.
    if look.holiday != Holiday::Xmas && look.lights < 0.5 {
        return;
    }
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (bx, by) = cam.project(x, y, z);
    let height = 46.0f32;
    let base_w = 13.0f32;
    // The cone, dark green with a lit left side.
    for i in 0..height as i32 {
        let t = i as f32 / height;
        let half = (base_w * (1.0 - t)).max(0.8);
        let y0 = by - i as f32;
        for dx in -(half as i32)..=(half as i32) {
            let lit = dx as f32 / half < -0.15;
            let c = if lit {
                palette::rgb(0x2e, 0x7a, 0x3e)
            } else {
                palette::rgb(0x1d, 0x52, 0x2c)
            };
            fb.put(bx as i32 + dx, y0 as i32, c);
        }
    }
    // Bulbs in a spiral up the cone.
    let mut k = 0usize;
    for i in 0..height as i32 / 3 {
        let t = i as f32 * 3.0 / height;
        let half = (base_w * (1.0 - t)).max(1.0);
        let ang = t * 9.0;
        let dx = (ang.cos() * half).round() as i32;
        let y0 = by - i as f32 * 3.0 - 2.0;
        fb.put(bx as i32 + dx, y0 as i32, palette::with_alpha(look.bulb(k), 0xFE));
        k += 1;
    }
    // A gold star on top.
    let ty = by - height - 2.0;
    let star = palette::with_alpha(pal::WINDOW_LIT, 0xFE);
    fb.put(bx as i32, ty as i32, star);
    fb.put(bx as i32 - 1, ty as i32, star);
    fb.put(bx as i32 + 1, ty as i32, star);
    fb.put(bx as i32, ty as i32 - 1, star);
    fb.put(bx as i32, ty as i32 + 1, star);
}

/// A banner strung between two posts over the square (B53: the Bills, and the
/// personal banner date). Blue and red bands in the Bills look; the garland
/// colours otherwise. Drawn at the tile `(x, y)` and raised above the ground.
pub fn banner_sign(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, look: &HolidayLook) {
    if !look.holiday.has_banner() {
        return;
    }
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (bx, by) = cam.project(x, y, z);
    let (bx, by) = (bx as i32, by as i32 - 30);
    let (band_a, band_b) = if look.holiday.is_bills() {
        (palette::rgb(0x1f, 0x3c, 0x88), palette::rgb(0xc8, 0x1e, 0x2e))
    } else {
        (look.bulb(0), look.bulb(1 % look.bulbs.max(1)))
    };
    let post = palette::rgb(0x5a, 0x4a, 0x3a);

    // The Bills banner is twice the size of the personal one, so it reads at 1x.
    let (hw, hh) = if look.holiday.is_bills() { (22i32, 8i32) } else { (9, 4) };
    let band = (2 * hw + 1) / 6;
    fb.line((bx as f32 - hw as f32, by as f32), (bx as f32 - hw as f32, by as f32 + hh as f32 + 3.0), post);
    fb.line((bx as f32 + hw as f32, by as f32), (bx as f32 + hw as f32, by as f32 + hh as f32 + 3.0), post);
    for dx in -hw..=hw {
        for dy in 0..hh {
            let c = if dy == 0 || dy == hh - 1 {
                pal::WINDOW_LIT
            } else if (dx + hw) / band.max(1) % 2 == 0 {
                band_a
            } else {
                band_b
            };
            fb.put(bx + dx, by + dy, c);
        }
    }
}

/// Tailgaters at the stadium lot (B53): a grill with embers and smoke, and
/// three figures in blue or red jerseys. `(x, y)` is the grill's tile; `seed`
/// varies the crowd from one grill to the next.
pub fn tailgate(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, look: &HolidayLook, seed: u32) {
    if !look.holiday.is_bills() {
        return;
    }
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (gx, gy) = cam.project(x, y, z);
    let (gx, gy) = (gx as i32, gy as i32);
    let steel = palette::rgb(0x33, 0x33, 0x38);
    for dx in 0..3 {
        for dy in 0..2 {
            fb.put(gx + dx, gy + dy, steel);
        }
    }
    fb.put(gx + 1, gy - 1, palette::rgb(0xff, 0x6a, 0x1a));
    fb.put(gx + 1, gy - 2, palette::rgb(0x9a, 0x9a, 0xa0));
    let skin = palette::rgb(0xe8, 0xb8, 0x90);
    let blue = palette::rgb(0x1f, 0x3c, 0x88);
    let red = palette::rgb(0xc8, 0x1e, 0x2e);
    for i in 0..3i32 {
        let fx = gx + 5 + i * 3 + ((seed >> i) & 1) as i32;
        let jersey = if (i as u32 + seed) % 2 == 0 { blue } else { red };
        fb.put(fx, gy - 3, skin);
        fb.put(fx, gy - 2, jersey);
        fb.put(fx, gy - 1, jersey);
        fb.put(fx, gy, steel);
    }
}

/// Fireworks over the city (B36): bursts at midnight on New Year's Eve and at
/// night on the 4th of July. Deterministic from `t` (seconds into the display)
/// and the seed, so a still frame is reproducible.
pub fn fireworks(fb: &mut Framebuffer, w: usize, h: usize, t: f32, seed: u64, look: &HolidayLook) {
    if !look.holiday.is_night_fireworks() {
        return;
    }
    // B36b: big bursts — 40..120 px radius at 1920 wide, four at a time, each
    // with trails, a bright core and a faint reflection on the water below the
    // horizon.
    let scale = (w as f32 / 1920.0).max(0.5);
    let period = 2.2f32;
    for b in 0..4 {
        let phase = (t / period + b as f32 * 0.29).fract();
        let burst_seed = seed ^ ((t / period) as u64 + b as u64 * 7919);
        let cx = 0.14 + 0.72 * tex::hash1((burst_seed as u32).wrapping_mul(7)) as f32;
        let cy = 0.06 + 0.20 * tex::hash1((burst_seed as u32).wrapping_mul(11)) as f32;
        let (cx, cy) = (cx * w as f32, cy * h as f32);
        let max_r = (60.0 + 60.0 * tex::hash1((burst_seed as u32).wrapping_mul(13))) * scale;
        let colour = look.bulb(b + (burst_seed as usize % 4));
        burst(fb, w, h, cx, cy, max_r, phase, burst_seed, colour, scale, true);
    }
}

/// B53: the Bills win fireworks, over the stadium (or the biggest park when the
/// city has no stadium). Only the disc above the host is drawn into, so the
/// rest of the frame is left alone. `(cx, cy)` is the host's centre on screen,
/// `span` its width in pixels.
pub fn fireworks_over(
    fb: &mut Framebuffer,
    cx: f32,
    cy: f32,
    span: f32,
    t: f32,
    seed: u64,
    look: &HolidayLook,
) {
    if look.holiday != Holiday::BillsWin {
        return;
    }
    let (w, h) = (fb.w, fb.h);
    let scale = (span / 120.0).clamp(0.5, 2.0);
    let period = 2.2f32;
    for b in 0..2 {
        let phase = (t / period + b as f32 * 0.5).fract();
        let burst_seed = seed ^ ((t / period) as u64 + b as u64 * 104_729);
        // Bursts rise from above the host, inside a disc about the span wide.
        let ang = tex::hash1((burst_seed as u32).wrapping_mul(5)) as f32 * std::f32::consts::TAU;
        let dist = 0.6 * span * tex::hash1((burst_seed as u32).wrapping_mul(9)) as f32;
        let bx = cx + ang.cos() * dist;
        let by = cy - span * 0.9 - ang.sin().abs() * dist * 0.5;
        let max_r = span * (0.35 + 0.2 * tex::hash1((burst_seed as u32).wrapping_mul(13)) as f32);
        let colour = look.bulb(b + (burst_seed as usize % 4));
        burst(fb, w, h, bx, by, max_r, phase, burst_seed, colour, scale, false);
    }
}

/// One firework: three shells of sparks with trails, a flash at the shell
/// and, when `reflect`, a smear on the water below the horizon.
#[allow(clippy::too_many_arguments)]
fn burst(
    fb: &mut Framebuffer,
    w: usize,
    h: usize,
    cx: f32,
    cy: f32,
    max_r: f32,
    phase: f32,
    burst_seed: u64,
    colour: Color,
    scale: f32,
    reflect: bool,
) {
    let radius = max_r * (phase * 2.2).min(1.0);
    let fade = (1.0 - phase).max(0.0).powf(0.75);
    // Three shells: an outer ring of sparks, a middle one and a hot core.
    for (ring, n, rk) in [(0usize, 44usize, 1.0f32), (1, 26, 0.62), (2, 14, 0.3)] {
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU
                + tex::hash1(burst_seed as u32 + ring as u32 * 31 + i as u32) * 0.6;
            let r = radius * rk
                * (0.78 + 0.34 * tex::hash1(burst_seed as u32 ^ ((i as u32) << 3) ^ ring as u32));
            let x = cx + a.cos() * r;
            let y = cy + a.sin() * r * 0.72 + 10.0 * phase * phase * scale;
            let c = if ring == 2 { palette::rgb(0xff, 0xf2, 0xd8) } else { colour };
            // A trail behind each spark, fading back toward the centre.
            for k in 0..4 {
                let tr = 1.0 - k as f32 * 0.06;
                let px = (cx + a.cos() * r * tr) as i32;
                let py = (cy + a.sin() * r * 0.72 * tr + 10.0 * phase * phase * scale) as i32;
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                fb.put_blended(px, py, c, ((0.9 - k as f32 * 0.2) * fade).clamp(0.0, 1.0));
            }
            let px = x as i32;
            let py = y as i32;
            if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                fb.put_blended(px, py, c, (0.95 * fade + 0.05).min(1.0));
                fb.put_blended(px + 1, py, c, (0.6 * fade).min(1.0));
            }
        }
    }
    // The shell's flash.
    if phase < 0.10 {
        for dy in -3..=3 {
            for dx in -3..=3 {
                if dx * dx + dy * dy <= 9 {
                    fb.put_blended(cx as i32 + dx, cy as i32 + dy, palette::rgb(0xff, 0xf4, 0xd0), 1.0);
                }
            }
        }
    }
    if !reflect {
        return;
    }
    // The reflection on the water: a vertical smear just below the horizon.
    let horizon = (h as f32 * 0.30) as i32;
    for k in 0..(h as i32 / 8).min(90) {
        let yy = horizon + k;
        if yy < 0 || yy as usize >= h {
            continue;
        }
        let a = (0.20 * fade) * (1.0 - k as f32 / 90.0);
        let half = (radius * 0.35) as i32;
        for dx in -half..=half {
            let px = cx as i32 + dx;
            if px >= 0 && (px as usize) < w && (k as f32) < radius * 0.5 {
                fb.put_blended(px, yy, colour, a);
            }
        }
    }
}

/// A market stall: a striped awning over a table, on the plaza (B36).
pub fn market_stall(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, v: u8) {
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (bx, by) = cam.project(x, y, z);
    let (bx, by) = (bx as i32, by as i32);
    let cloth = [
        palette::rgb(0xd8, 0x3a, 0x3a),
        palette::rgb(0x2f, 0x6a, 0xc8),
        palette::rgb(0x2f, 0x9a, 0x5a),
        palette::rgb(0xd8, 0xa8, 0x2a),
    ][v as usize % 4];
    // Table.
    fb.fill_rect(bx - 5, by - 2, 11, 2, palette::rgb(0x8a, 0x6a, 0x46));
    // Posts.
    fb.fill_rect(bx - 5, by - 9, 1, 7, palette::rgb(0x6a, 0x52, 0x3a));
    fb.fill_rect(bx + 5, by - 9, 1, 7, palette::rgb(0x6a, 0x52, 0x3a));
    // Striped awning.
    for i in 0..11 {
        let c = if i % 2 == 0 { cloth } else { palette::rgb(0xf0, 0xec, 0xdc) };
        fb.fill_rect(bx - 5 + i, by - 10, 1, 2, c);
    }
    // Produce on the table.
    let produce = [pal::FLOWERS[0], pal::FLOWERS[2], pal::LEAF[0], pal::FLOWERS[1]];
    for (i, c) in produce.iter().enumerate() {
        fb.put(bx - 3 + i as i32 * 2, by - 3, *c);
    }
}

/// A parade float: a cart with a coloured canopy and a couple of figures.
pub fn parade_float(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, v: u8) {
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (bx, by) = cam.project(x, y, z);
    let (bx, by) = (bx as i32, by as i32);
    let cloth = [
        palette::rgb(0xd8, 0x3a, 0x3a),
        palette::rgb(0x2f, 0x9a, 0x5a),
        palette::rgb(0xd8, 0xa8, 0x2a),
        palette::rgb(0x8a, 0x4a, 0xc8),
    ][v as usize % 4];
    // Cart.
    fb.fill_rect(bx - 8, by - 4, 17, 4, palette::rgb(0xb0, 0x8a, 0x5a));
    fb.fill_rect(bx - 8, by - 4, 17, 1, palette::rgb(0xd8, 0xb0, 0x7a));
    fb.fill_rect(bx - 6, by - 1, 2, 2, pal::CAR_WHEEL);
    fb.fill_rect(bx + 5, by - 1, 2, 2, pal::CAR_WHEEL);
    // Canopy.
    for i in 0..17 {
        fb.fill_rect(bx - 8 + i, by - 12, 1, 1, if i % 3 == 0 { palette::rgb(0xf0, 0xec, 0xdc) } else { cloth });
    }
    for i in 0..17 {
        fb.put(bx - 8 + i, by - 11, palette::shade(cloth, 0.8));
    }
    // Two figures on the float.
    for (i, dx) in [-3i32, 3].iter().enumerate() {
        let c = pal::PEOPLE_SHIRT[(v as usize + i) % pal::PEOPLE_SHIRT.len()];
        fb.fill_rect(bx + dx, by - 8, 1, 3, c);
        fb.put(bx + dx, by - 9, pal::PEOPLE_HEAD);
    }
}

/// Floodlights and a lit crowd for a stadium game night (B36).
pub fn stadium_lights(fb: &mut Framebuffer, cam: &Camera, x: i32, y: i32, w: i32, h: i32, level: u8, glow: f32) {
    if glow < 0.3 {
        return;
    }
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let corners = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    for (cx, cy) in corners {
        let p = cam.project(cx as f32, cy as f32, z + 26);
        // A mast and a bright head.
        let base = cam.project(cx as f32, cy as f32, z);
        fb.line(base, p, palette::rgb(0x6a, 0x6a, 0x72));
        fb.fill_rect(p.0 as i32 - 1, p.1 as i32 - 1, 3, 2, pal::WINDOW_LIT);
        // A cone of light down onto the pitch.
        for i in 1..20 {
            let t = i as f32 / 20.0;
            let px = p.0 + (base.0 - p.0) * t * 0.9;
            let py = p.1 + (base.1 - p.1) * t;
            let half = 1.0 + t * 5.0;
            for dx in -(half as i32)..=(half as i32) {
                fb.put_blended(px as i32 + dx, py as i32, pal::WINDOW_LIT, 0.10 * (1.0 - t));
            }
        }
    }
    // The crowd: a ring of tiny figures along the near stand, lit.
    let a = cam.project(x as f32, (y + h) as f32, z + 2);
    let b = cam.project((x + w) as f32, (y + h) as f32, z + 2);
    let steps = (w * 6).max(6);
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let px = a.0 + (b.0 - a.0) * t;
        let py = a.1 + (b.1 - a.1) * t;
        let c = pal::PEOPLE_SHIRT[(i as usize) % pal::PEOPLE_SHIRT.len()];
        fb.put(px as i32, py as i32 - 1, c);
        fb.put(px as i32, py as i32 - 2, pal::PEOPLE_HEAD);
    }
    let _ = level_px(0.0);
}

/// A park or plaza lamp (B38): a slim post with a warm head and a pool of light
/// on the ground around it. Only at night.
pub fn park_lamp(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, glow: f32) {
    if glow < 0.4 {
        return;
    }
    let z = level as i32 * crate::iso::HEIGHT_UNIT;
    let (px, py) = cam.project(x, y, z);
    let (px, py) = (px as i32, py as i32);
    let h = 10;
    fb.fill_rect(px, py - h, 1, h, palette::rgb(0x2a, 0x2c, 0x30));
    fb.fill_rect(px - 1, py - h - 2, 3, 2, pal::LAMP_GLOW);
    fb.put_blended(px, py - h - 3, pal::LAMP_GLOW, 0.9);
    // The pool on the ground, an iso ellipse.
    let strength = ((glow - 0.4) / 0.6).clamp(0.0, 1.0);
    // Dithered, not blended: an exact colour survives the night tint (the
    // lamp light is emissive), a blend of it would be darkened (B38).
    for dy in -5i32..=5 {
        for dx in -9i32..=9 {
            let d = ((dx * dx) as f32 / 81.0 + (dy * dy) as f32 / 25.0).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0) * 0.75 * strength;
            if a > 0.02 && tex::hash2(px + dx, py + dy, 0x51) < a {
                fb.put(px + dx, py + dy, pal::LAMP_GLOW);
            }
        }
    }
}

/// A porch light on a house at night (B38): a warm dot by the door with a small
/// halo, on most houses in the evening and few after midnight.
pub fn porch_light(fb: &mut Framebuffer, cam: &Camera, x: i32, y: i32, w: i32, h: i32, level: u8, front: u8, glow: f32, seed: u32) {
    if glow < 0.35 {
        return;
    }
    // After midnight only a few are still on (D25).
    let late = !(18.0..=23.9).contains(&(glow * 100.0).min(23.9));
    let on = (seed.wrapping_mul(2654435761) >> 16) % 100;
    let threshold = if late { 18 } else { 65 };
    if on >= threshold {
        return;
    }
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (fx, fy) = match front {
        0 => ((x + w) as f32, y as f32 + h as f32 * 0.5),
        1 => (x as f32 + w as f32 * 0.5, (y + h) as f32),
        2 => (x as f32, y as f32 + h as f32 * 0.5),
        _ => (x as f32 + w as f32 * 0.5, y as f32),
    };
    let p = cam.project(fx, fy, base + 4);
    let (px, py) = (p.0 as i32, p.1 as i32);
    fb.put(px, py, pal::LAMP_GLOW);
    fb.put(px + 1, py, pal::LAMP_GLOW);
    fb.put(px, py - 1, pal::LAMP_GLOW);
    fb.put(px + 1, py - 1, pal::LAMP_GLOW);
    fb.put(px - 1, py, pal::LAMP_GLOW);
}

/// A soft round puff of a colour: smoke, dust. Blended, so it layers over the
/// ground and over itself without a hard edge.
fn blob(fb: &mut Framebuffer, cx: f32, cy: f32, rx: f32, ry: f32, c: Color, alpha: f32) {
    let (x0, y0) = ((cx - rx).floor() as i32, (cy - ry).floor() as i32);
    let (x1, y1) = ((cx + rx).ceil() as i32, (cy + ry).ceil() as i32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5 - cx) / rx.max(0.5);
            let dy = (y as f32 + 0.5 - cy) / ry.max(0.5);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= 1.0 {
                fb.put_blended(x, y, c, alpha * (1.0 - d * 0.3));
            }
        }
    }
}

/// A settler, a chopper, a carrier or the wagon (B39/D62, B40: readable at 1×).
/// `phase` swings the axe (choppers) and the legs (walkers).
pub fn settler(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &map::TileMap,
    x: f32,
    y: f32,
    kind: u8,
    phase: f32,
) {
    let z = crate::iso::level_px(map.surface_level_at(x, y));
    let (sx, sy) = cam.project(x, y, z);
    let (px, py) = (sx as i32, sy as i32);
    let ink = pal::OUTLINE;
    if kind == 1 {
        // The wagon: a plank bed, a canvas hoop, two big wheels, a driver.
        tex::ellipse(fb, px as f32, py as f32 + 1.0, 12.0, 2.5, palette::rgb(0x3a, 0x44, 0x2e));
        // Bed and side boards.
        fb.fill_rect(px - 10, py - 6, 21, 5, ink);
        fb.fill_rect(px - 9, py - 5, 19, 3, palette::rgb(0x8a, 0x5e, 0x36));
        fb.fill_rect(px - 9, py - 5, 19, 1, palette::rgb(0xb0, 0x7e, 0x4a));
        // The canvas cover: an arch of pale cloth, shaded on the far side.
        for i in 0..8 {
            let half = 9 - (i as f32 * 0.3) as i32;
            let c = if i < 3 {
                palette::rgb(0xf2, 0xec, 0xd8)
            } else if i < 6 {
                palette::rgb(0xdc, 0xd4, 0xbc)
            } else {
                palette::rgb(0xc0, 0xb6, 0x9e)
            };
            fb.fill_rect(px - half, py - 8 - i, half * 2 + 1, 1, c);
        }
        fb.fill_rect(px - 9, py - 8, 19, 1, ink);
        // Hoop ribs.
        for k in [-6, 0, 6] {
            fb.line(
                (px as f32 + k as f32, py as f32 - 7.0),
                (px as f32 + k as f32, py as f32 - 13.0),
                palette::rgb(0xa8, 0x9e, 0x86),
            );
        }
        // Wheels: dark tyres with a pale hub.
        for wx in [px - 7, px + 7] {
            fb.fill_rect(wx - 3, py - 3, 7, 6, ink);
            fb.fill_rect(wx - 2, py - 2, 5, 4, palette::rgb(0x3e, 0x36, 0x2c));
            fb.fill_rect(wx - 1, py - 1, 3, 2, palette::rgb(0x9a, 0x8a, 0x6e));
        }
        // The driver on the box.
        fb.fill_rect(px + 8, py - 10, 4, 4, palette::rgb(0x4a, 0x5e, 0x8a));
        fb.fill_rect(px + 8, py - 13, 4, 3, pal::PEOPLE_HEAD);
        fb.fill_rect(px + 8, py - 13, 4, 1, palette::rgb(0x4a, 0x36, 0x28));
        return;
    }
    // A person, 5 px wide and 12 tall: a shadow, two legs, a coat with arms, a
    // head. Outlined, so the figure reads against grass and dirt alike.
    tex::ellipse(fb, px as f32, py as f32 + 1.0, 3.5, 1.4, palette::rgb(0x3a, 0x44, 0x2e));
    let coat = pal::PEOPLE_SHIRT[(kind as usize * 2 + 1) % pal::PEOPLE_SHIRT.len()];
    let step = if phase < 0.5 { 0 } else { 1 };
    fb.fill_rect(px - 1, py - 4, 1, 4, pal::PEOPLE_LEGS);
    fb.fill_rect(px + 1, py - 4 + step, 1, 4 - step, pal::PEOPLE_LEGS);
    // Coat: dark edge on the left, lit on the right, arms at both sides.
    fb.fill_rect(px - 2, py - 10, 5, 6, ink);
    fb.fill_rect(px - 1, py - 9, 3, 4, coat);
    fb.fill_rect(px - 1, py - 9, 1, 4, palette::shade(coat, 0.7));
    fb.fill_rect(px + 2, py - 9, 1, 4, palette::shade(coat, 1.2));
    fb.fill_rect(px - 2, py - 9, 1, 3, palette::shade(coat, 0.82));
    // Head: 3 px of skin with a row of hair (no box outline — it read as a hat).
    fb.fill_rect(px - 1, py - 14, 3, 1, palette::rgb(0x46, 0x34, 0x26));
    fb.fill_rect(px - 1, py - 13, 3, 3, pal::PEOPLE_HEAD);
    fb.put(px + 1, py - 12, palette::rgb(0xc0, 0x96, 0x74));
    if kind == 2 {
        // A chopper with an axe: the swing is the phase (up, down, up).
        let swing = (phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let hand = (px as f32 + 2.0, py as f32 - 8.0);
        let tip = (hand.0 + 2.0 + swing * 3.0, hand.1 - 8.0 + swing * 10.0);
        fb.line(hand, tip, palette::rgb(0x6a, 0x4e, 0x34));
        fb.line(
            (tip.0 - 1.0, tip.1 - 1.0),
            (tip.0 + 2.0, tip.1 - 1.0),
            palette::rgb(0xd8, 0xdc, 0xe4),
        );
    }
    if kind == 3 {
        // A carrier with a log across the chest (not over the head, where it
        // read as a hat brim).
        fb.fill_rect(px - 4, py - 9, 11, 3, ink);
        fb.fill_rect(px - 3, py - 8, 9, 1, palette::rgb(0x9a, 0x6e, 0x44));
        fb.put(px - 3, py - 8, palette::rgb(0xe0, 0xc0, 0x8c));
        fb.put(px + 5, py - 8, palette::rgb(0xe0, 0xc0, 0x8c));
    }
}

/// The settlers' camp: canvas tents and a campfire (B39 item 2, B40 size). Smoke
/// by day, a warm glow by night.
pub fn camp(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &map::TileMap,
    fx: i32,
    fy: i32,
    tents: &[(i32, i32)],
    fire: (i32, i32),
    glow: f32,
    t: f32,
) {
    let ink = pal::OUTLINE;
    for (dx, dy) in tents {
        let (x, y) = (fx + dx, fy + dy);
        let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
        let (sx, sy) = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
        let (px, py) = (sx as f32, sy as f32);
        // Ground: a soft dark patch, not a slab.
        blob(fb, px, py + 1.0, 15.0, 4.0, palette::rgb(0x3a, 0x44, 0x2e), 0.35);
        // A ridge tent: a lit near face and a shaded far face meeting at a ridge
        // pole, with a dark door and two guy ropes. ~28 px wide, 18 tall at 1×.
        let apex = (px, py - 18.0);
        let (l, r) = ((px - 13.0, py), (px + 13.0, py));
        // The lit near face, and a shaded strip down the right side so the
        // canvas has two planes.
        fb.fill_poly(&[apex, l, r], palette::rgb(0xee, 0xe4, 0xcc));
        fb.fill_poly(
            &[apex, (px + 5.0, py - 7.0), (px + 9.0, py - 2.0), r],
            palette::rgb(0xb0, 0xa4, 0x88),
        );
        // A seam down the middle of the lit face.
        fb.line(apex, (px - 2.0, py - 1.0), palette::rgb(0xd8, 0xcc, 0xb0));
        // Outline: the two slopes and the base.
        fb.line(apex, l, ink);
        fb.line(apex, r, ink);
        fb.line(l, r, ink);
        // The ridge pole peeking above the apex.
        fb.line(apex, (px - 3.0, py - 20.0), palette::rgb(0x8a, 0x7e, 0x68));
        // The door: a small dark-brown flap, low in the near face.
        fb.fill_poly(
            &[(px - 3.0, py - 1.0), (px - 2.0, py - 7.0), (px + 2.0, py - 6.0), (px + 3.0, py - 1.0)],
            palette::rgb(0x4a, 0x40, 0x34),
        );
        // Guy ropes and pegs.
        fb.line(apex, (px - 19.0, py + 2.0), palette::rgb(0x9a, 0x92, 0x7e));
        fb.line(apex, (px + 19.0, py + 2.0), palette::rgb(0x9a, 0x92, 0x7e));
        fb.put(px as i32 - 19, py as i32 + 2, palette::rgb(0x6a, 0x5e, 0x48));
        fb.put(px as i32 + 19, py as i32 + 2, palette::rgb(0x6a, 0x5e, 0x48));
    }
    // The fire: a stone ring, crossed logs, a flame, and smoke or glow.
    let (x, y) = (fx + fire.0, fy + fire.1);
    let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let (sx, sy) = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let (px, py) = (sx as i32, sy as i32);
    // Ring of stones (a small iso ring, 18 px across).
    for (dx, dy) in [
        (-8, 0), (-6, 2), (-2, 4), (3, 4), (7, 2), (8, 0), (-7, -1), (-2, -3), (3, -3), (7, -1),
    ] {
        fb.fill_rect(px + dx - 1, py + dy - 1, 4, 3, palette::rgb(0x5e, 0x5a, 0x52));
        fb.fill_rect(px + dx, py + dy - 1, 2, 1, palette::rgb(0x9e, 0x9a, 0x90));
    }
    // Logs and flame.
    fb.line(
        (px as f32 - 5.0, py as f32 - 2.0),
        (px as f32 + 5.0, py as f32 - 4.0),
        palette::rgb(0x54, 0x3a, 0x24),
    );
    fb.line(
        (px as f32 - 4.0, py as f32 - 4.0),
        (px as f32 + 5.0, py as f32 - 2.0),
        palette::rgb(0x7a, 0x56, 0x34),
    );
    let flick = (t * 6.0).sin() * 0.8 + (t * 11.0).cos() * 0.5;
    let h = 9.0 + flick;
    for i in 0..(h as i32).max(3) {
        let w = (5.0 - i as f32 * 0.45).max(1.0) as i32;
        let c = if i < 3 {
            palette::rgb(0xff, 0xf0, 0xb0)
        } else if i < 6 {
            palette::rgb(0xff, 0xc0, 0x50)
        } else {
            palette::rgb(0xe8, 0x78, 0x2c)
        };
        fb.fill_rect(px - w / 2, py - 4 - i, w, 1, c);
    }
    if glow > 0.4 {
        // A warm pool round the fire at night.
        for dy in -6i32..=6 {
            for dx in -11i32..=11 {
                let d = ((dx * dx) as f32 / 121.0 + (dy * dy) as f32 / 36.0).sqrt();
                let a = (1.0 - d).clamp(0.0, 1.0) * 0.55 * glow;
                if a > 0.02 {
                    fb.put_blended(px + dx, py + dy, pal::LAMP_GLOW, a);
                }
            }
        }
    } else {
        // Smoke by day: a column of puffs rising and drifting.
        for i in 0..7 {
            let rise = i as f32 * 5.0 + (t * 3.0).rem_euclid(5.0);
            let drift = (rise * 0.3).sin() * 4.0;
            let r = 1.5 + i as f32 * 0.6;
            blob(
                fb,
                px as f32 + drift + rise * 0.3,
                py as f32 - 11.0 - rise,
                r,
                r * 0.8,
                palette::rgb(0xdc, 0xdc, 0xd8),
                (0.5 - i as f32 * 0.055).max(0.05),
            );
        }
    }
}

/// A log pile or a stump left by the felling (B39 item 3, B40 size).
pub fn log_pile(fb: &mut Framebuffer, cam: &Camera, map: &map::TileMap, x: i32, y: i32, kind: u8) {
    let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let (sx, sy) = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let (px, py) = (sx as i32, sy as i32);
    let ink = pal::OUTLINE;
    let bark = palette::rgb(0x7a, 0x52, 0x32);
    let bark_lit = palette::rgb(0x9a, 0x6c, 0x42);
    let cut = palette::rgb(0xe0, 0xc0, 0x8c);
    if kind == 0 {
        // A pile of cut logs: three courses of two, dark bark, pale sawn ends.
        tex::ellipse(fb, px as f32, py as f32 + 1.0, 11.0, 3.0, palette::rgb(0x2c, 0x30, 0x26));
        for row in 0..3 {
            let yy = py - 2 - row * 4;
            let inset = row as i32;
            let n = 2 + (row == 0) as i32;
            for k in 0..n {
                let xx = px - 8 + inset + k * 9;
                fb.fill_rect(xx, yy, 9, 4, ink);
                fb.fill_rect(xx + 1, yy + 1, 7, 2, if k % 2 == 0 { bark } else { bark_lit });
                fb.fill_rect(xx + 1, yy + 1, 1, 2, cut);
                fb.fill_rect(xx + 7, yy + 1, 1, 2, cut);
            }
        }
    } else {
        // A stump: a bark ring, a pale cut top, and a root flare.
        tex::ellipse(fb, px as f32, py as f32 + 1.0, 7.0, 2.0, palette::rgb(0x2c, 0x30, 0x26));
        fb.fill_rect(px - 5, py - 5, 11, 5, ink);
        fb.fill_rect(px - 4, py - 4, 9, 4, bark);
        fb.fill_rect(px - 4, py - 4, 9, 1, bark_lit);
        tex::ellipse(fb, px as f32, py as f32 - 5.0, 5.0, 2.2, ink);
        tex::ellipse(fb, px as f32, py as f32 - 5.0, 4.0, 1.6, cut);
        fb.put(px, py - 5, palette::rgb(0xb0, 0x8a, 0x5e));
    }
}

/// A tree going over (B40): the trunk tips through `frac` (0 upright, 1 down),
/// the canopy swings with it, and dust puffs where it lands.
pub fn falling_tree(
    fb: &mut Framebuffer,
    cam: &Camera,
    map: &map::TileMap,
    x: i32,
    y: i32,
    frac: f32,
    dir: i8,
    size: u8,
    variant: u8,
) {
    let z = crate::iso::level_px(map.surface_level_at(x as f32 + 0.5, y as f32 + 0.5));
    let (sx, sy) = cam.project(x as f32 + 0.5, y as f32 + 0.5, z);
    let (px, py) = (sx as f32, sy as f32);
    let len = 14.0 + size as f32 * 5.0;
    let angle = frac * 1.45; // ~83°, so it ends lying on the ground
    let (sn, cs) = angle.sin_cos();
    let d = dir as f32;
    let tip = (px + sn * len * d, py - cs * len);
    let trunk = palette::rgb(0x6a, 0x4a, 0x30);
    let ink = pal::OUTLINE;
    // Trunk: three passes for a 4 px width, dark on the shaded side.
    let dark = palette::shade(trunk, 0.7);
    fb.line((px + 2.0, py), (tip.0 + 2.0, tip.1), dark);
    fb.line((px + 1.0, py), (tip.0 + 1.0, tip.1), trunk);
    fb.line((px, py), tip, trunk);
    fb.line((px - 1.0, py - 1.0), (tip.0 - 1.0, tip.1), palette::shade(trunk, 1.25));
    // Canopy: three blobs down the last half of the trunk.
    let greens = [
        palette::rgb(0x4e, 0x7a, 0x3c),
        palette::rgb(0x3e, 0x66, 0x30),
        palette::rgb(0x62, 0x8e, 0x46),
    ];
    let r = 7.0 + size as f32 * 2.0;
    for (k, f) in [0.55f32, 0.8, 1.0].into_iter().enumerate() {
        let cx = px + (tip.0 - px) * f;
        let cy = py + (tip.1 - py) * f - 2.0;
        let rr = r * (1.0 - k as f32 * 0.15);
        // A dark green rim, not black ink: an ink ellipse read as a hole.
        let rim = palette::shade(greens[(k + variant as usize) % 3], 0.55);
        tex::ellipse(fb, cx, cy + 1.0, rr, rr * 0.8, rim);
        tex::ellipse(fb, cx, cy, rr - 1.0, rr * 0.8 - 1.0, greens[(k + variant as usize) % 3]);
        tex::ellipse(
            fb,
            cx - rr * 0.3,
            cy - rr * 0.35,
            rr * 0.45,
            rr * 0.3,
            palette::shade(greens[(k + variant as usize) % 3], 1.18),
        );
    }
    // Dust where it lands.
    if frac > 0.82 {
        let a = ((frac - 0.82) / 0.18).min(1.0);
        for k in 0..5 {
            let ox = (k as f32 - 2.0) * 5.0 * d;
            blob(
                fb,
                tip.0 + ox,
                tip.1 + 1.0 - k as f32 * 0.6,
                3.0 + k as f32,
                1.6,
                palette::rgb(0xcf, 0xc4, 0xa8),
                0.45 * a,
            );
        }
    }
}
