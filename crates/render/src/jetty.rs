//! B79: the jetties of the islands: a plank jetty, a pier with rails and a lamp,
//! a harbour pier with a hut, crates and bollards. Static art, painted into the
//! scene's static layer (like the harbour), one pass per jetty.

use crate::framebuffer::Framebuffer;
use crate::iso::{Camera, HEIGHT_UNIT};
use crate::palette::{self, rgb, Color};
use map::TileMap;

const PLANK: Color = rgb(0xb4, 0x92, 0x64);
const PLANK_LIT: Color = rgb(0xc8, 0xa8, 0x78);
const PLANK_DARK: Color = rgb(0x6a, 0x50, 0x38);
const POST: Color = rgb(0x4e, 0x3a, 0x2a);
const ROPE: Color = rgb(0xd8, 0xc4, 0x90);

/// One jetty as the renderer needs it (all primitives, so the fixtures compare
/// cheaply).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct JettyLook {
    /// The built deck tiles, from the shore out.
    pub deck: Vec<(i32, i32)>,
    pub dir: (i32, i32),
    /// 1 jetty, 2 pier, 3 harbour.
    pub level: u8,
    /// The land tile the deck starts from.
    pub shore: (i32, i32),
}

pub fn draw_jetty(fb: &mut Framebuffer, cam: &Camera, map: &TileMap, j: &JettyLook, glow: f32) {
    if j.deck.is_empty() {
        return;
    }
    let (dx, dy) = (j.dir.0 as f32, j.dir.1 as f32);
    let (px, py) = (dy, dx);
    // Half width of the deck across, in tiles.
    let hw = match j.level {
        1 => 0.24,
        2 => 0.34,
        _ => 0.42,
    };
    let lift = 3;
    let z_at = |x: f32, y: f32| (map.surface_level_at(x, y) * HEIGHT_UNIT as f32).round() as i32 + lift;
    let p = |u: f32, v: f32, z: i32| cam.project(u, v, z);
    let n = j.deck.len();
    // From the shore edge of the first tile to the end of the last.
    let (sx0, sy0) = (j.deck[0].0 as f32, j.deck[0].1 as f32);
    let start = (sx0 + 0.5 - dx * 0.5, sy0 + 0.5 - dy * 0.5);
    let end = (sx0 + 0.5 + dx * (n as f32 - 0.5), sy0 + 0.5 + dy * (n as f32 - 0.5));
    let zc = z_at(j.deck[n - 1].0 as f32 + 0.5, j.deck[n - 1].1 as f32 + 0.5);
    let corner = |at: (f32, f32), s: f32| (at.0 + px * hw * s, at.1 + py * hw * s);
    // The water under it: a soft dark shadow, a few px down.
    let sh = [
        p(corner(start, -1.0).0, corner(start, -1.0).1, zc - 5),
        p(corner(end, -1.0).0, corner(end, -1.0).1, zc - 5),
        p(corner(end, 1.0).0, corner(end, 1.0).1, zc - 5),
        p(corner(start, 1.0).0, corner(start, 1.0).1, zc - 5),
    ];
    for i in 0..3 {
        let mut q = sh;
        for v in q.iter_mut() {
            v.1 += i as f32;
        }
        shade_poly(fb, &q, 0.1);
    }
    // Posts down into the water, two rows, a tile apart.
    for i in 0..=n {
        let t = i as f32;
        for s in [-1.0f32, 1.0] {
            let at = (start.0 + dx * t, start.1 + dy * t);
            let c = corner(at, s * 0.9);
            let top = p(c.0, c.1, zc);
            let bot = p(c.0, c.1, zc - 9);
            fb.line(top, bot, POST);
            let b2 = (bot.0 + 1.0, bot.1);
            fb.line((top.0 + 1.0, top.1), b2, palette::shade(POST, 0.7));
            // A foam ring where the post meets the water.
            fb.put_blended(bot.0 as i32 - 1, bot.1 as i32, rgb(0xf0, 0xf8, 0xfc), 0.5);
            fb.put_blended(bot.0 as i32 + 1, bot.1 as i32, rgb(0xf0, 0xf8, 0xfc), 0.35);
        }
    }
    // The deck: one surface, with the near edge a plank thicker.
    let top = [
        p(corner(start, -1.0).0, corner(start, -1.0).1, zc),
        p(corner(end, -1.0).0, corner(end, -1.0).1, zc),
        p(corner(end, 1.0).0, corner(end, 1.0).1, zc),
        p(corner(start, 1.0).0, corner(start, 1.0).1, zc),
    ];
    fb.fill_poly(&top, PLANK);
    // The side facing the camera: a dark fascia.
    let side = if px > 0.0 || py > 0.0 { 1.0 } else { -1.0 };
    let (a, b) = (corner(start, side), corner(end, side));
    fb.fill_poly(&[p(a.0, a.1, zc), p(b.0, b.1, zc), p(b.0, b.1, zc - 2), p(a.0, a.1, zc - 2)], PLANK_DARK);
    // Plank lines across the deck, every quarter tile, alternating tone.
    let steps = (n as f32 * 4.0) as i32;
    for k in 1..steps {
        let t = k as f32 / 4.0 - 0.5;
        let at = (sx0 + 0.5 + dx * t, sy0 + 0.5 + dy * t);
        let (l, r) = (corner(at, -1.0), corner(at, 1.0));
        let c = if k % 2 == 0 { PLANK_DARK } else { PLANK_LIT };
        fb.line(p(l.0, l.1, zc), p(r.0, r.1, zc), palette::mix(PLANK, c, 0.55));
    }
    // A bright edge along the lit side.
    let (a2, b2) = (corner(start, -side), corner(end, -side));
    fb.line(p(a2.0, a2.1, zc), p(b2.0, b2.1, zc), PLANK_LIT);
    // Level 2 and 3: a railing on both sides, posts every tile.
    if j.level >= 2 {
        let rail = rgb(0xe4, 0xdc, 0xc8);
        for s in [-1.0f32, 1.0] {
            let mut prev: Option<(f32, f32)> = None;
            for i in 1..=n {
                let t = i as f32 - 0.5;
                let at = (sx0 + 0.5 + dx * (t - 0.5 + 0.5), sy0 + 0.5 + dy * (t - 0.5 + 0.5));
                let c = corner(at, s * 0.96);
                let foot = p(c.0, c.1, zc);
                let head = (foot.0, foot.1 - 6.0);
                fb.line(foot, head, rail);
                if let Some(pv) = prev {
                    fb.line(pv, head, rail);
                }
                prev = Some(head);
            }
        }
    }
    // The tip: a bollard and a coil of rope; a lamp on the pier and harbour.
    let tipc = (end.0 - dx * 0.25, end.1 - dy * 0.25);
    let bc = corner(tipc, 0.55);
    let b = p(bc.0, bc.1, zc);
    fb.fill_rect(b.0 as i32 - 1, b.1 as i32 - 3, 3, 3, rgb(0x3a, 0x3c, 0x40));
    fb.put(b.0 as i32, b.1 as i32 - 4, rgb(0x56, 0x58, 0x5e));
    let rc = corner(tipc, -0.45);
    let r = p(rc.0, rc.1, zc);
    fb.fill_rect(r.0 as i32 - 1, r.1 as i32 - 1, 3, 2, ROPE);
    fb.put(r.0 as i32, r.1 as i32 - 1, palette::shade(ROPE, 0.8));
    if j.level >= 2 {
        let lc = corner(tipc, -0.9);
        let l = p(lc.0, lc.1, zc);
        fb.line(l, (l.0, l.1 - 15.0), rgb(0x3a, 0x3c, 0x40));
        fb.fill_rect(l.0 as i32 - 1, l.1 as i32 - 18, 3, 3, rgb(0xf0, 0xe6, 0xb4));
        if glow > 0.35 {
            for (ox, oy, c) in [(0, 0, 0.9), (-2, 0, 0.3), (2, 0, 0.3), (0, -2, 0.3), (0, 2, 0.3)] {
                fb.put_blended(l.0 as i32 + ox, l.1 as i32 - 17 + oy, rgb(0xff, 0xe0, 0xa0), c);
            }
        }
    }
    if j.level >= 3 {
        // A small hut at the shore end, crates and a second lamp.
        let hc = (sx0 + 0.5 + dx * 0.55 - px * 0.0, sy0 + 0.5 + dy * 0.55);
        let hc = corner(hc, 0.1);
        let (w, d) = (0.34, 0.34);
        crate::transit::box_at(fb, cam, hc.0 - w * 0.5, hc.1 - d * 0.5, zc, w, d, 10, rgb(0x6a, 0x4a, 0x3a), rgb(0xb0, 0x9a, 0x7c), rgb(0x8a, 0x76, 0x5c));
        crate::transit::box_at(fb, cam, hc.0 - w * 0.5 - 0.04, hc.1 - d * 0.5 - 0.04, zc + 10, w + 0.08, d + 0.08, 3, rgb(0xa0, 0x4a, 0x3a), rgb(0x8a, 0x3e, 0x30), rgb(0x6e, 0x30, 0x26));
        let wc = p(hc.0 + 0.1, hc.1 + d * 0.5 + 0.02, zc + 6);
        fb.fill_rect(wc.0 as i32, wc.1 as i32, 3, 3, if glow > 0.35 { rgb(0xff, 0xdc, 0x90) } else { rgb(0x3a, 0x5a, 0x7a) });
        for (k, col) in [(0.0f32, rgb(0x8a, 0x6a, 0x3a)), (0.22, rgb(0x9a, 0x78, 0x44))] {
            let cc = corner((sx0 + 0.5 + dx * (1.3 + k), sy0 + 0.5 + dy * (1.3 + k)), -0.55);
            crate::transit::box_at(fb, cam, cc.0 - 0.09, cc.1 - 0.09, zc, 0.18, 0.18, 4, palette::shade(col, 1.15), col, palette::shade(col, 0.75));
        }
    }
}

fn shade_poly(fb: &mut Framebuffer, q: &[(f32, f32); 4], cov: f32) {
    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for v in q {
        x0 = x0.min(v.0);
        x1 = x1.max(v.0);
        y0 = y0.min(v.1);
        y1 = y1.max(v.1);
    }
    // Scanline over the bounding box, testing the quad by half-planes.
    for y in y0.floor() as i32..=y1.ceil() as i32 {
        for x in x0.floor() as i32..=x1.ceil() as i32 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut inside = true;
            let mut sign = 0.0f32;
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                let cr = (b.0 - a.0) * (fy - a.1) - (b.1 - a.1) * (fx - a.0);
                if cr != 0.0 {
                    if sign == 0.0 {
                        sign = cr.signum();
                    } else if cr.signum() != sign {
                        inside = false;
                        break;
                    }
                }
            }
            if inside {
                fb.put_blended(x, y, rgb(0x08, 0x1c, 0x2a), cov);
            }
        }
    }
}
