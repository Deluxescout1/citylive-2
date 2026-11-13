//! Attractions (B62, D75): the clock tower, the zoo, the castle; and the
//! gameday lot. Engine art, original: nothing here copies an existing place.

use super::*;

const STONE: Color = palette::rgb(0xa8, 0xa4, 0x9c);
const SLATE: Color = palette::rgb(0x4a, 0x56, 0x72);

/// A round tower: a shaded cylinder from `z0` to `z1` px above `base`, `r` px
/// wide at the radius, centred on tile `(x, y)`.
#[allow(clippy::too_many_arguments)]
pub fn cylinder(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, r_px: i32, base: i32, z0: i32, z1: i32, c: Color) {
    let (cx, cy) = cam.project(x, y, base);
    let (cx, cy) = (cx as i32, cy as i32);
    for dx in -r_px..=r_px {
        let u = dx as f32 / r_px as f32; // -1 (lit) .. 1 (shade)
        let lit = 1.12 - (u + 1.0) * 0.30;
        // The barrel's top and bottom edges bow with the ellipse of the base.
        let bow = ((1.0 - u * u).max(0.0).sqrt() * r_px as f32 * 0.5) as i32;
        let col = palette::shade(c, lit);
        for yy in (cy - z1 + bow)..=(cy - z0 + bow) {
            // Mortar courses every 4 px.
            let course = (yy - cy + z1) % 4 == 0;
            fb.put(cx + dx, yy, if course { palette::shade(col, 0.88) } else { col });
        }
        // The top face.
        fb.put(cx + dx, cy - z1 + bow - 1, palette::shade(c, 1.1));
    }
}

/// A conical roof over a round tower.
pub fn cone(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, r_px: i32, base: i32, z0: i32, z1: i32, c: Color) {
    let (cx, cy) = cam.project(x, y, base);
    let (cx, cy) = (cx as i32, cy as i32);
    let rr = r_px + 2;
    for dx in -rr..=rr {
        let u = dx as f32 / rr as f32;
        let h = ((1.0 - u.abs()) * (z1 - z0) as f32) as i32;
        let bow = ((1.0 - u * u).max(0.0).sqrt() * rr as f32 * 0.5) as i32;
        let lit = 1.12 - (u + 1.0) * 0.32;
        let col = palette::shade(c, lit);
        for k in 0..=h {
            let yy = cy - z0 + bow - k;
            fb.put(cx + dx, yy, if k % 4 == 3 { palette::shade(col, 0.88) } else { col });
        }
    }
    // The finial.
    fb.line((cx as f32, (cy - z1) as f32), (cx as f32, (cy - z1 - 5) as f32), palette::rgb(0x40, 0x3a, 0x34));
}

// ---- clock tower -----------------------------------------------------------

/// Where a tower's clock faces are on screen (left and right faces), given the
/// tile position of its south-east corner area.
pub fn clock_faces(cam: &Camera, x: f32, y: f32, level: u8) -> [(f32, f32); 2] {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let hw = TOWER_HALF;
    [
        cam.project(x, y + hw, base + CLOCK_Z),
        cam.project(x + hw, y, base + CLOCK_Z),
    ]
}

const TOWER_HALF: f32 = 0.26;
const CLOCK_Z: i32 = 68;

/// A clock tower in the square: plinth, stone shaft with slit windows, the
/// clock stage with two blank faces (the hands are drawn per frame by
/// [`clock_hands`]), a belfry and a slate spire.
pub fn clock_tower(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let hw = TOWER_HALF;
    let (x0, y0, x1, y1) = (x - hw, y - hw, x + hw, y + hw);
    // Plinth.
    iso_box(fb, cam, (x0 - 0.06, y0 - 0.06, x1 + 0.06, y1 + 0.06), base, 0, 7, palette::shade(STONE, 0.9));
    // Shaft.
    iso_box(fb, cam, (x0, y0, x1, y1), base, 7, 58, STONE);
    // Slit windows on both visible faces.
    for z in [20, 34, 47] {
        let l = cam.project(x, y1, base + z);
        let r = cam.project(x1, y, base + z);
        for q in [l, r] {
            fb.fill_rect(q.0 as i32, q.1 as i32 - 3, 1, 5, palette::rgb(0x2a, 0x2a, 0x38));
        }
    }
    // The clock stage: slightly wider, with a pale face on each side.
    iso_box(fb, cam, (x0 - 0.04, y0 - 0.04, x1 + 0.04, y1 + 0.04), base, 58, 78, palette::shade(STONE, 1.06));
    for (fx, fy) in clock_faces(cam, x, y, level) {
        let (cx, cy) = (fx as i32, fy as i32 + 1);
        for dy in -7..=7i32 {
            for dx in -7..=7i32 {
                let d2 = dx * dx + dy * dy;
                if d2 <= 49 {
                    let rim = d2 > 36;
                    fb.put(cx + dx, cy + dy, if rim { palette::rgb(0x6a, 0x54, 0x30) } else { palette::rgb(0xf0, 0xea, 0xd4) });
                }
            }
        }
        // Hour ticks.
        for k in 0..12 {
            let a = k as f32 / 12.0 * std::f32::consts::TAU;
            fb.put(cx + (a.sin() * 5.0).round() as i32, cy - (a.cos() * 5.0).round() as i32, palette::rgb(0x3a, 0x34, 0x2c));
        }
    }
    // Belfry: open arches, dark.
    iso_box(fb, cam, (x0 - 0.02, y0 - 0.02, x1 + 0.02, y1 + 0.02), base, 78, 92, palette::shade(STONE, 0.98));
    for (ax, ay) in [(x, y1 + 0.02), (x1 + 0.02, y)] {
        let q = cam.project(ax, ay, base + 85);
        fb.fill_rect(q.0 as i32 - 2, q.1 as i32 - 4, 5, 8, palette::rgb(0x24, 0x24, 0x30));
        fb.put(q.0 as i32 - 1, q.1 as i32 - 5, palette::rgb(0x24, 0x24, 0x30));
        fb.put(q.0 as i32 + 1, q.1 as i32 - 5, palette::rgb(0x24, 0x24, 0x30));
        fb.put(q.0 as i32, q.1 as i32 - 1, palette::rgb(0xd0, 0xa8, 0x40));
    }
    // The spire: a four-sided slate pyramid.
    let apex = cam.project(x, y, base + 118);
    let c = [
        cam.project(x0 - 0.04, y0 - 0.04, base + 92),
        cam.project(x1 + 0.04, y0 - 0.04, base + 92),
        cam.project(x1 + 0.04, y1 + 0.04, base + 92),
        cam.project(x0 - 0.04, y1 + 0.04, base + 92),
    ];
    fb.fill_poly(&[apex, c[2], c[3]], palette::shade(SLATE, 0.92));
    fb.fill_poly(&[apex, c[1], c[2]], palette::shade(SLATE, 0.66));
    fb.line(c[3], c[2], palette::shade(SLATE, 0.6));
    fb.line(c[2], c[1], palette::shade(SLATE, 0.5));
    fb.line(apex, (apex.0, apex.1 - 6.0), palette::rgb(0xd0, 0xa8, 0x40));
    fb.put(apex.0 as i32 + 1, apex.1 as i32 - 6, palette::rgb(0xd0, 0xa8, 0x40));
}

/// The hands for the time of day `secs` (seconds since midnight), on both faces.
pub fn clock_hands(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, secs: f32) {
    let mins = (secs / 60.0).floor();
    let hour_a = ((mins / 60.0) % 12.0) / 12.0 * std::f32::consts::TAU;
    let min_a = (mins % 60.0) / 60.0 * std::f32::consts::TAU;
    for (fx, fy) in clock_faces(cam, x, y, level) {
        let (cx, cy) = (fx, fy + 1.0);
        fb.line((cx, cy), (cx + hour_a.sin() * 3.0, cy - hour_a.cos() * 3.0), palette::rgb(0x20, 0x1c, 0x18));
        fb.line((cx, cy), (cx + min_a.sin() * 5.0, cy - min_a.cos() * 5.0), palette::rgb(0x20, 0x1c, 0x18));
    }
}

// ---- zoo -------------------------------------------------------------------

/// A low fence round `(x0, y0)-(x1, y1)` on the ground: posts and two rails.
fn fence(fb: &mut Framebuffer, cam: &Camera, (x0, y0, x1, y1): (f32, f32, f32, f32), base: i32, wood: Color) {
    let h = 5;
    let pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)];
    for w in pts.windows(2) {
        let a = cam.project(w[0].0, w[0].1, base + h);
        let b = cam.project(w[1].0, w[1].1, base + h);
        fb.line(a, b, wood);
        let a2 = cam.project(w[0].0, w[0].1, base + 2);
        let b2 = cam.project(w[1].0, w[1].1, base + 2);
        fb.line(a2, b2, palette::shade(wood, 0.85));
    }
    for &(x, y) in &pts[..4] {
        let a = cam.project(x, y, base);
        let b = cam.project(x, y, base + h + 1);
        fb.line(a, b, palette::shade(wood, 0.72));
    }
}

fn diamond(fb: &mut Framebuffer, cam: &Camera, (x0, y0, x1, y1): (f32, f32, f32, f32), base: i32, c: Color) {
    let q = [cam.project(x0, y0, base), cam.project(x1, y0, base), cam.project(x1, y1, base), cam.project(x0, y1, base)];
    fb.fill_poly(&q, c);
}

fn speckle(fb: &mut Framebuffer, cam: &Camera, (x0, y0, x1, y1): (f32, f32, f32, f32), base: i32, c: Color, salt: i32) {
    for i in 0..26 {
        let f = tex::hash1((salt * 97 + i) as u32);
        let g = tex::hash1((salt * 31 + i * 7 + 3) as u32);
        let p = cam.project(x0 + (x1 - x0) * f as f32, y0 + (y1 - y0) * g as f32, base);
        fb.put(p.0 as i32, p.1 as i32, c);
    }
}

fn elephant(fb: &mut Framebuffer, x: i32, y: i32) {
    let g = palette::rgb(0x88, 0x88, 0x92);
    let d = palette::shade(g, 0.78);
    let l = palette::shade(g, 1.15);
    // Body.
    for dy in -12..=-3 {
        let half = if dy < -10 { 6 } else if dy > -5 { 7 } else { 8 };
        for dx in -half..=half {
            fb.put(x + dx, y + dy, if dy < -9 { l } else if dy > -5 { d } else { g });
        }
    }
    // Legs.
    for lx in [-6, -3, 2, 5] {
        for dy in -3..=0 {
            fb.put(x + lx, y + dy, d);
            fb.put(x + lx + 1, y + dy, g);
        }
    }
    // Head, ear, trunk, tusk.
    for dy in -13..=-6 {
        for dx in 7..=11 {
            fb.put(x + dx, y + dy, g);
        }
    }
    for dy in -9..=-3 {
        fb.put(x + 11, y + dy, g);
        fb.put(x + 12, y + dy + 1, d);
    }
    fb.put(x + 10, y - 7, palette::rgb(0xf0, 0xec, 0xd8));
    fb.put(x + 11, y - 6, palette::rgb(0xf0, 0xec, 0xd8));
    for dy in -12..=-7 {
        fb.put(x + 6, y + dy, d);
        fb.put(x + 5, y + dy, d);
    }
    fb.put(x + 9, y - 11, palette::rgb(0x18, 0x18, 0x1c));
}

fn giraffe(fb: &mut Framebuffer, x: i32, y: i32) {
    let tan = palette::rgb(0xe0, 0xb0, 0x50);
    let spot = palette::rgb(0x8a, 0x52, 0x20);
    // Body and legs.
    for dy in -12..=-7 {
        for dx in -5..=3 {
            let c = if (dx + dy * 2_i32).rem_euclid(5) == 0 { spot } else { tan };
            fb.put(x + dx, y + dy, c);
        }
    }
    for lx in [-5, -3, 1, 3] {
        for dy in -6..=0 {
            fb.put(x + lx, y + dy, tan);
        }
    }
    // The neck, tall, with spots, and a small head.
    for dy in -28..=-11 {
        for dx in 3..=5 {
            let c = if (dx + dy + 0_i32).rem_euclid(4) == 0 { spot } else { tan };
            fb.put(x + dx, y + dy, c);
        }
    }
    for dx in 4..=8 {
        fb.put(x + dx, y - 29, tan);
        fb.put(x + dx, y - 28, tan);
    }
    fb.put(x + 6, y - 30, spot);
    fb.put(x + 4, y - 30, spot);
    fb.put(x + 7, y - 29, palette::rgb(0x18, 0x14, 0x10));
    fb.put(x - 5, y - 10, spot);
    fb.put(x - 6, y - 9, spot);
}

fn zebra(fb: &mut Framebuffer, x: i32, y: i32) {
    let w = palette::rgb(0xf0, 0xee, 0xe8);
    let k = palette::rgb(0x24, 0x24, 0x2c);
    for dy in -9..=-4 {
        for dx in -6..=4 {
            fb.put(x + dx, y + dy, if (dx + 20) % 3 == 0 { k } else { w });
        }
    }
    for lx in [-6, -4, 2, 4] {
        for dy in -3..=0 {
            fb.put(x + lx, y + dy, if dy % 2 == 0 { k } else { w });
        }
    }
    for dy in -13..=-8 {
        for dx in 4..=5 {
            fb.put(x + dx, y + dy, if dy % 2 == 0 { k } else { w });
        }
    }
    for dx in 4..=8 {
        fb.put(x + dx, y - 14, w);
        fb.put(x + dx, y - 13, if dx == 8 { k } else { w });
    }
    fb.put(x + 4, y - 15, k);
    for dy in -9..=-6 {
        fb.put(x - 7, y + dy, k);
    }
}

fn lion(fb: &mut Framebuffer, x: i32, y: i32) {
    let tan = palette::rgb(0xd8, 0xa8, 0x58);
    let mane = palette::rgb(0x7a, 0x44, 0x1c);
    for dy in -6..=-2 {
        for dx in -6..=3 {
            fb.put(x + dx, y + dy, tan);
        }
    }
    for lx in [-6, -4, 1, 3] {
        fb.put(x + lx, y - 1, palette::shade(tan, 0.8));
        fb.put(x + lx, y, palette::shade(tan, 0.8));
    }
    for dy in -9..=-3 {
        for dx in 2..=8 {
            if (dx - 5) * (dx - 5) + (dy + 6) * (dy + 6) <= 12 {
                fb.put(x + dx, y + dy, mane);
            }
        }
    }
    fb.put(x + 5, y - 6, tan);
    fb.put(x + 6, y - 6, tan);
    fb.put(x + 5, y - 5, tan);
    fb.put(x + 6, y - 5, palette::rgb(0x18, 0x14, 0x10));
    for k in 0..5 {
        fb.put(x - 7 - k, y - 4 + k / 3, tan);
    }
    fb.put(x - 11, y - 3, mane);
}

fn penguins(fb: &mut Framebuffer, x: i32, y: i32) {
    for (dx, dy) in [(-6, 0), (0, -2), (6, 1)] {
        let (px, py) = (x + dx, y + dy);
        for k in 0..6 {
            fb.put(px, py - k, palette::rgb(0x22, 0x24, 0x2c));
            fb.put(px + 1, py - k, palette::rgb(0x22, 0x24, 0x2c));
        }
        fb.put(px, py - 2, palette::rgb(0xf4, 0xf4, 0xf0));
        fb.put(px, py - 3, palette::rgb(0xf4, 0xf4, 0xf0));
        fb.put(px + 1, py - 5, palette::rgb(0xf0, 0xa0, 0x30));
    }
}

fn flamingos(fb: &mut Framebuffer, x: i32, y: i32) {
    for (dx, dy, up) in [(-7, 0, true), (0, 1, false), (7, -1, true)] {
        let (px, py) = (x + dx, y + dy);
        let pink = palette::rgb(0xf0, 0x7a, 0x9a);
        for k in 0..6 {
            fb.put(px, py - k, palette::shade(pink, 0.9));
        }
        for dxx in -1..=1 {
            fb.put(px + dxx, py - 7, pink);
            fb.put(px + dxx, py - 8, pink);
        }
        let head = if up { 11 } else { 9 };
        for k in 8..head {
            fb.put(px + 1, py - k, pink);
        }
        fb.put(px + 2, py - head, pink);
        fb.put(px + 3, py - head, palette::rgb(0x24, 0x20, 0x24));
    }
}

/// The zoo: a lawn with a gravel path, a gate and fenced paddocks, each with
/// its animals and trees. `seed` mixes which animals are in which paddock.
#[allow(clippy::too_many_arguments)]
pub fn zoo(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, w: f32, h: f32, level: u8, seed: u32, lawn: Color) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (x0, y0, x1, y1) = (x + 0.12, y + 0.12, x + w - 0.12, y + h - 0.12);
    diamond(fb, cam, (x0, y0, x1, y1), base, lawn);
    speckle(fb, cam, (x0, y0, x1, y1), base, palette::shade(lawn, 1.12), seed as i32);
    let gravel = palette::rgb(0xcc, 0xbc, 0x98);
    // A central path along the long axis and a cross path.
    let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    diamond(fb, cam, (cx - 0.16, y0, cx + 0.16, y1), base, gravel);
    diamond(fb, cam, (x0, cy - 0.16, x1, cy + 0.16), base, gravel);
    // Paddocks in the four quadrants: (savanna, dry, water/ice, grass).
    let quads = [
        ((x0 + 0.14, y0 + 0.14, cx - 0.26, cy - 0.26), 0u32),
        ((cx + 0.26, y0 + 0.14, x1 - 0.14, cy - 0.26), 1),
        ((x0 + 0.14, cy + 0.26, cx - 0.26, y1 - 0.14), 2),
        ((cx + 0.26, cy + 0.26, x1 - 0.14, y1 - 0.14), 3),
    ];
    let wood = palette::rgb(0x8a, 0x66, 0x42);
    let pick = (seed % 3) as usize;
    for (qi, (r, _)) in quads.iter().enumerate() {
        let (a0, b0, a1, b1) = *r;
        if a1 - a0 < 0.5 || b1 - b0 < 0.5 {
            continue;
        }
        let kind = (qi + pick) % 4;
        let ground = match kind {
            0 => palette::rgb(0xc8, 0xb0, 0x70),
            1 => palette::rgb(0x9a, 0x8a, 0x64),
            2 => palette::rgb(0x58, 0xa8, 0xd0),
            _ => palette::shade(lawn, 0.92),
        };
        diamond(fb, cam, *r, base, ground);
        speckle(fb, cam, *r, base, palette::shade(ground, 0.84), 11 + qi as i32);
    }
    // Fences go over the ground; animals and trees go over the fences.
    for (r, _) in &quads {
        if r.2 - r.0 >= 0.5 && r.3 - r.1 >= 0.5 {
            fence(fb, cam, *r, base, wood);
        }
    }
    for (qi, (r, _)) in quads.iter().enumerate() {
        let (a0, b0, a1, b1) = *r;
        if a1 - a0 < 0.5 || b1 - b0 < 0.5 {
            continue;
        }
        let kind = (qi + pick) % 4;
        let c = cam.project((a0 + a1) * 0.5, (b0 + b1) * 0.5 + 0.1, base);
        let (px, py) = (c.0 as i32, c.1 as i32);
        match kind {
            0 => {
                elephant(fb, px - 6, py + 2);
                zebra(fb, px + 10, py + 6);
            }
            1 => {
                giraffe(fb, px, py + 2);
                lion(fb, px + 14, py + 7);
            }
            2 => penguins(fb, px, py + 3),
            _ => flamingos(fb, px, py + 3),
        }
        // A tree in the paddock's far corner.
        let t = cam.project(a0 + 0.2, b0 + 0.2, base);
        let (tx, ty) = (t.0 as i32, t.1 as i32);
        fb.fill_rect(tx, ty - 8, 1, 8, palette::rgb(0x5a, 0x42, 0x2c));
        for dy in -6..=1i32 {
            for dx in -4..=4i32 {
                if dx * dx + dy * dy * 2 <= 16 {
                    fb.put(tx + dx, ty - 12 + dy, if (dx + dy) % 3 == 0 { palette::rgb(0x4a, 0x9a, 0x40) } else { palette::rgb(0x3a, 0x84, 0x36) });
                }
            }
        }
    }
    // The gate at the front corner: two pillars and a sign arch.
    let g0 = cam.project(cx - 0.45, y1, base);
    let g1 = cam.project(cx + 0.45, y1, base);
    for g in [g0, g1] {
        fb.fill_rect(g.0 as i32 - 1, g.1 as i32 - 14, 3, 14, palette::rgb(0xb8, 0x88, 0x50));
        fb.fill_rect(g.0 as i32 - 2, g.1 as i32 - 15, 5, 2, palette::rgb(0x8a, 0x5e, 0x32));
    }
    let (ax, ay) = (g0.0 as i32, g0.1 as i32 - 14);
    let span = (g1.0 - g0.0) as i32;
    for dx in 0..=span {
        let sag = ((dx as f32 / span.max(1) as f32 - 0.5).powi(2) * 4.0) as i32;
        fb.put(ax + dx, ay + sag, palette::rgb(0x8a, 0x5e, 0x32));
        fb.put(ax + dx, ay + sag + 1, palette::rgb(0xf0, 0xc8, 0x30));
        fb.put(ax + dx, ay + sag + 2, palette::rgb(0xf0, 0xc8, 0x30));
        fb.put(ax + dx, ay + sag + 3, palette::rgb(0xd0, 0x90, 0x20));
    }
    // A paw-print on the sign: three toes and a pad.
    let mx = ax + span / 2;
    for (dx, dy) in [(-2, 3), (0, 2), (2, 3), (0, 4)] {
        fb.put(mx + dx, ay + dy + 1, palette::rgb(0x4a, 0x2e, 0x16));
    }
}

// ---- castle ----------------------------------------------------------------

/// A castle on its plot: a curtain wall with crenellations, four round towers
/// with conical roofs, a gate on the front and a keep inside. `(x, y, w, h)`
/// is the plot in tiles.
#[allow(clippy::too_many_arguments)]
pub fn castle(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, w: f32, h: f32, level: u8, seed: u32, ground: Color) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (x0, y0, x1, y1) = (x + 0.35, y + 0.35, x + w - 0.35, y + h - 0.35);
    let wall_t = 0.26;
    let wall_h = 26;
    // Courtyard cobbles and an outer lawn ring.
    diamond(fb, cam, (x + 0.05, y + 0.05, x + w - 0.05, y + h - 0.05), base, palette::shade(ground, 0.96));
    diamond(fb, cam, (x0, y0, x1, y1), base, palette::rgb(0x9a, 0x94, 0x88));
    for i in 0..60 {
        let f = tex::hash1(seed.wrapping_add(i * 3));
        let g = tex::hash1(seed.wrapping_add(i * 5 + 1));
        let p = cam.project(x0 + (x1 - x0) * f as f32, y0 + (y1 - y0) * g as f32, base);
        fb.put(p.0 as i32, p.1 as i32, palette::rgb(0x84, 0x7e, 0x74));
    }
    let top_wall = |fb: &mut Framebuffer, r: (f32, f32, f32, f32)| {
        iso_box(fb, cam, r, base, 0, wall_h, STONE);
        // Crenellations: little blocks along the top edge, the near edges.
        let (a0, b0, a1, b1) = r;
        let n = (((a1 - a0).max(b1 - b0)) / 0.2) as i32;
        for i in (0..n).step_by(2) {
            let f = i as f32 / n.max(1) as f32;
            let (px, py) = if (a1 - a0) > (b1 - b0) {
                (a0 + (a1 - a0) * f, b1 - 0.03)
            } else {
                (a1 - 0.03, b0 + (b1 - b0) * f)
            };
            let p = cam.project(px, py, base + wall_h);
            fb.fill_rect(p.0 as i32, p.1 as i32 - 3, 3, 3, palette::shade(STONE, 1.05));
            fb.put(p.0 as i32, p.1 as i32 - 3, palette::shade(STONE, 1.2));
        }
    };
    // Far walls first.
    top_wall(fb, (x0, y0, x1, y0 + wall_t));
    top_wall(fb, (x0, y0, x0 + wall_t, y1));
    // Back towers.
    let tr = 20;
    let tower = |fb: &mut Framebuffer, tx: f32, ty: f32| {
        cylinder(fb, cam, tx, ty, tr, base, 0, 58, palette::shade(STONE, 1.02));
        // A slit window.
        let p = cam.project(tx, ty + 0.05, base + 34);
        fb.fill_rect(p.0 as i32 - 1, p.1 as i32, 2, 5, palette::rgb(0x2a, 0x2a, 0x38));
        cone(fb, cam, tx, ty, tr, base, 58, 92, SLATE);
    };
    tower(fb, x0, y0);
    tower(fb, x1, y0);
    tower(fb, x0, y1);
    // The keep, in the middle: a square block with a taller corner turret.
    let (kx, ky) = ((x0 + x1) * 0.5 - 0.15, (y0 + y1) * 0.5 - 0.15);
    iso_box(fb, cam, (kx - 0.62, ky - 0.62, kx + 0.62, ky + 0.62), base, 0, 70, palette::shade(STONE, 0.96));
    iso_box(fb, cam, (kx - 0.68, ky - 0.68, kx + 0.68, ky + 0.68), base, 70, 76, palette::shade(STONE, 1.08));
    for q in [-0.38f32, 0.0, 0.38] {
        for (px, py, z) in [(kx + q, ky + 0.62, 40), (kx + 0.62, ky + q, 40)] {
            let p = cam.project(px, py, base + z);
            fb.fill_rect(p.0 as i32 - 1, p.1 as i32 - 4, 3, 6, palette::rgb(0x24, 0x28, 0x3a));
        }
    }
    // The great door.
    let dpt = cam.project(kx, ky + 0.62, base + 7);
    fb.fill_rect(dpt.0 as i32 - 3, dpt.1 as i32 - 8, 7, 10, palette::rgb(0x5a, 0x3a, 0x22));
    fb.fill_rect(dpt.0 as i32 - 2, dpt.1 as i32 - 9, 5, 1, palette::rgb(0x5a, 0x3a, 0x22));
    cylinder(fb, cam, kx + 0.5, ky - 0.5, 14, base, 70, 98, palette::shade(STONE, 1.04));
    cone(fb, cam, kx + 0.5, ky - 0.5, 14, base, 98, 130, palette::rgb(0x8a, 0x3a, 0x34));
    // A banner on the turret, in the castle's own colours.
    let fp = cam.project(kx + 0.5, ky - 0.5, base + 130);
    for k in 0..8 {
        let sway = ((k as f32) * 0.5).sin().round() as i32;
        for dy in 0..4 {
            fb.put(fp.0 as i32 + 1 + k, fp.1 as i32 - 6 + dy + sway, if dy % 2 == 0 { palette::rgb(0xc8, 0x30, 0x34) } else { palette::rgb(0xf0, 0xc8, 0x30) });
        }
    }
    // Near walls: the right wall, then the front wall with its gate.
    top_wall(fb, (x1 - wall_t, y0, x1, y1));
    let gate_half = 0.42;
    let cxg = (x0 + x1) * 0.5;
    top_wall(fb, (x0, y1 - wall_t, cxg - gate_half, y1));
    top_wall(fb, (cxg + gate_half, y1 - wall_t, x1, y1));
    // The gatehouse: a lintel over the opening and a dark arch with portcullis.
    iso_box(fb, cam, (cxg - gate_half, y1 - wall_t, cxg + gate_half, y1), base, 17, wall_h + 6, palette::shade(STONE, 1.04));
    let gp = cam.project(cxg, y1, base);
    fb.fill_rect(gp.0 as i32 - 8, gp.1 as i32 - 17, 17, 17, palette::rgb(0x2a, 0x26, 0x30));
    for k in 0..4 {
        fb.fill_rect(gp.0 as i32 - 7 + k * 4, gp.1 as i32 - 17, 1, 11, palette::rgb(0x4a, 0x44, 0x40));
    }
    tower(fb, x1, y1);
}

// ---- the gameday lot ---------------------------------------------------------

pub const BLUE: Color = palette::rgb(0x1f, 0x4c, 0xc0);
pub const RED: Color = palette::rgb(0xd0, 0x22, 0x30);

/// A tailgate lot: canopies in the two team colours with a grill and a table
/// under each, and pickups parked behind. Tile `(x, y)` is the lot's centre and
/// `span` its width in tiles. People are added by the scene's crowd.
#[allow(clippy::too_many_arguments)]
pub fn tailgate_lot(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, span: f32, level: u8, t: f32, seed: u32) {
    let n = ((span / 1.5) as i32).clamp(2, 5);
    for i in 0..n {
        let f = (i as f32 + 0.5) / n as f32;
        let tx = x - span * 0.5 + span * f;
        let (a, b) = if (i + seed as i32) % 2 == 0 { (BLUE, palette::rgb(0xf4, 0xf4, 0xf4)) } else { (RED, palette::rgb(0xf4, 0xf4, 0xf4)) };
        // A pickup behind each canopy, nose alternating.
        pickup(fb, cam, tx - 0.05, y - 0.85, level, if i % 2 == 0 { 1 } else { -1 }, [palette::rgb(0x3a, 0x5a, 0x7a), palette::rgb(0x8a, 0x2a, 0x2a), palette::rgb(0xc8, 0xc8, 0xc8)][(i as usize) % 3]);
        tent(fb, cam, tx, y, level, a, b);
        grill(fb, cam, tx + 0.6, y + 0.2, level, t + i as f32);
    }
}

// ---------------------------------------------------------------------------
// B52b: the government district's own buildings (engine art, per biome)
// ---------------------------------------------------------------------------

/// Which civic building to draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CivicBld {
    Courthouse,
    Police,
    PostOffice,
}

/// Stone, brick and roof colours in the world's style: pale limestone and slate
/// in the temperate land, sandstone and terracotta in the desert, grey granite
/// and dark slate under snow, whitewash and red tile in the tropics.
fn civic_palette(b: map::Biome, seed: u32) -> (Color, Color, Color) {
    use map::Biome::*;
    let (stone, brick, roof) = match b {
        Desert => ((0xdc, 0xbc, 0x8c), (0xc4, 0x8c, 0x5c), (0xa8, 0x5a, 0x3c)),
        Tundra => ((0xa4, 0xaa, 0xb2), (0x84, 0x58, 0x4c), (0x3c, 0x44, 0x54)),
        Tropical => ((0xf2, 0xee, 0xe2), (0xe4, 0xc8, 0x98), (0xb8, 0x50, 0x3c)),
        _ => ((0xd0, 0xca, 0xba), (0xa8, 0x62, 0x4a), (0x4a, 0x56, 0x72)),
    };
    let j = |c: (u8, u8, u8), k: i32| palette::rgb((c.0 as i32 + k).clamp(0, 255) as u8, (c.1 as i32 + k).clamp(0, 255) as u8, (c.2 as i32 + k).clamp(0, 255) as u8);
    let k = (seed % 9) as i32 - 4;
    (j(stone, k), j(brick, k), j(roof, 0))
}

/// A window quad on the +y face (`axis_y = true`, at `fixed` y) or the +x face.
#[allow(clippy::too_many_arguments)]
fn civic_pane(fb: &mut Framebuffer, cam: &Camera, base: i32, axis_y: bool, fixed: f32, a0: f32, a1: f32, z0: i32, z1: i32, c: Color) {
    let p = |a: f32, z: i32| if axis_y { cam.project(a, fixed, base + z) } else { cam.project(fixed, a, base + z) };
    fb.fill_poly(&[p(a0, z0), p(a1, z0), p(a1, z1), p(a0, z1)], c);
}

/// Draw a courthouse (columns, steps, pediment), a police station (blue sign
/// light, patrol cars out front) or a post office (flag, pillar box, van) on
/// its plot. The facade faces +y, toward the viewer.
#[allow(clippy::too_many_arguments)]
pub fn civic_building(fb: &mut Framebuffer, cam: &Camera, kind: CivicBld, x: f32, y: f32, w: f32, h: f32, level: u8, seed: u32, biome: map::Biome, glow: f32) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (stone, brick, roof) = civic_palette(biome, seed);
    let (x0, y0, x1, y1) = (x + 0.2, y + 0.2, x + w - 0.2, y + h - 0.2);
    let lit = glow > 0.35;
    let glass = if lit { palette::rgb(0xf4, 0xd0, 0x80) } else { palette::rgb(0x44, 0x5a, 0x78) };
    let dark = palette::rgb(0x2a, 0x2e, 0x3a);
    let pave = palette::rgb(0x8a, 0x88, 0x84);
    match kind {
        CivicBld::Courthouse => {
            diamond(fb, cam, (x + 0.05, y + 0.05, x + w - 0.05, y + h - 0.05), base, palette::shade(pave, 1.05));
            let body = (x0, y0, x1, y1 - 1.0);
            iso_box(fb, cam, body, base, 0, 30, stone);
            iso_box(fb, cam, (body.0 - 0.04, body.1 - 0.04, body.2 + 0.04, body.3 + 0.04), base, 30, 34, palette::shade(roof, 1.1));
            for i in 0..4 {
                let a = y0 + 0.35 + i as f32 * (y1 - 1.0 - y0 - 0.6) / 4.0;
                civic_pane(fb, cam, base, false, x1, a, a + 0.22, 12, 24, glass);
            }
            let (px0, px1) = (x0 + 0.45, x1 - 0.45);
            iso_box(fb, cam, (px0 - 0.1, y1 - 1.0, px1 + 0.1, y1 - 0.1), base, 0, 3, palette::shade(stone, 1.08));
            iso_box(fb, cam, (px0, y1 - 1.0, px1, y1 - 0.3), base, 3, 6, palette::shade(stone, 1.12));
            civic_pane(fb, cam, base, true, y1 - 0.9, (px0 + px1) * 0.5 - 0.22, (px0 + px1) * 0.5 + 0.22, 6, 20, dark);
            let n = 6;
            for i in 0..n {
                let cx = px0 + 0.1 + (px1 - px0 - 0.3) * i as f32 / (n - 1) as f32;
                iso_box(fb, cam, (cx, y1 - 0.85, cx + 0.14, y1 - 0.71), base, 6, 32, palette::shade(stone, 1.15));
            }
            iso_box(fb, cam, (px0 - 0.1, y1 - 0.95, px1 + 0.1, y1 - 0.62), base, 32, 37, palette::shade(stone, 1.05));
            let yy = y1 - 0.62;
            let p = |xx: f32, z: i32| cam.project(xx, yy, base + z);
            let mid = (px0 + px1) * 0.5;
            fb.fill_poly(&[p(px0 - 0.1, 37), p(px1 + 0.1, 37), p(mid, 50)], palette::shade(stone, 0.95));
            fb.fill_poly(&[p(px0 + 0.25, 38), p(px1 - 0.25, 38), p(mid, 47)], palette::shade(stone, 0.72));
            let cy = (y0 + y1 - 1.0) * 0.5;
            iso_box(fb, cam, (mid - 0.3, cy - 0.3, mid + 0.3, cy + 0.3), base, 34, 44, stone);
            let c = cam.project(mid, cy, base + 44);
            fb.fill_poly(&[(c.0 - 9.0, c.1), (c.0 + 9.0, c.1), (c.0, c.1 - 9.0)], roof);
        }
        CivicBld::Police => {
            diamond(fb, cam, (x + 0.05, y + 0.05, x + w - 0.05, y + h - 0.05), base, palette::shade(pave, 0.95));
            let body = (x0, y0, x1, y1 - 0.95);
            iso_box(fb, cam, body, base, 0, 26, brick);
            iso_box(fb, cam, (body.0 - 0.05, body.1 - 0.05, body.2 + 0.05, body.3 + 0.05), base, 26, 29, stone);
            for row in 0..2 {
                let z = 6 + row * 11;
                for i in 0..3 {
                    let a = x0 + 0.3 + i as f32 * (x1 - x0 - 0.5) / 3.0;
                    civic_pane(fb, cam, base, true, body.3, a, a + 0.28, z, z + 7, glass);
                }
                for i in 0..2 {
                    let a = y0 + 0.3 + i as f32 * (body.3 - y0 - 0.4) / 2.0;
                    civic_pane(fb, cam, base, false, x1, a, a + 0.26, z, z + 7, glass);
                }
            }
            let dm = (x0 + x1) * 0.5;
            civic_pane(fb, cam, base, true, body.3, dm - 0.18, dm + 0.18, 0, 10, dark);
            let sg = cam.project(dm, body.3 + 0.12, base + 17);
            let (sx, sy) = (sg.0 as i32, sg.1 as i32);
            fb.fill_rect(sx - 8, sy - 3, 16, 6, palette::rgb(0xf4, 0xf4, 0xf8));
            fb.fill_rect(sx - 8, sy - 3, 16, 1, palette::rgb(0x20, 0x20, 0x28));
            fb.fill_rect(sx - 8, sy + 2, 16, 1, palette::rgb(0x20, 0x20, 0x28));
            fb.fill_rect(sx - 6, sy - 1, 12, 2, palette::rgb(0x2c, 0x5c, 0xc8));
            let lamp = if lit { palette::rgb(0x70, 0xb8, 0xff) } else { palette::rgb(0x34, 0x64, 0xb0) };
            fb.fill_rect(sx - 2, sy - 7, 5, 4, lamp);
            fb.fill_rect(sx - 1, sy - 8, 3, 1, palette::shade(lamp, 1.3));
            for k in 0..2 {
                let cx = x0 + 0.3 + k as f32 * 1.1;
                let cy = y1 - 0.52;
                iso_box(fb, cam, (cx, cy, cx + 0.62, cy + 0.3), base, 0, 5, palette::rgb(0xf2, 0xf2, 0xf4));
                iso_box(fb, cam, (cx + 0.16, cy + 0.02, cx + 0.46, cy + 0.28), base, 5, 8, palette::rgb(0x22, 0x24, 0x2c));
                let l = cam.project(cx + 0.31, cy + 0.15, base + 9);
                fb.put(l.0 as i32 - 1, l.1 as i32, palette::rgb(0x40, 0x80, 0xff));
                fb.put(l.0 as i32 + 1, l.1 as i32, palette::rgb(0xff, 0x40, 0x40));
            }
        }
        CivicBld::PostOffice => {
            diamond(fb, cam, (x + 0.05, y + 0.05, x + w - 0.05, y + h - 0.05), base, palette::shade(pave, 1.02));
            let body = (x0, y0, x1, y1 - 0.9);
            iso_box(fb, cam, body, base, 0, 22, brick);
            iso_box(fb, cam, (body.0 - 0.06, body.1 - 0.06, body.2 + 0.06, body.3 + 0.06), base, 22, 26, roof);
            for i in 0..3 {
                let a = x0 + 0.3 + i as f32 * (x1 - x0 - 0.5) / 3.0;
                civic_pane(fb, cam, base, true, body.3, a, a + 0.3, 8, 17, glass);
            }
            for i in 0..2 {
                let a = y0 + 0.3 + i as f32 * (body.3 - y0 - 0.4) / 2.0;
                civic_pane(fb, cam, base, false, x1, a, a + 0.26, 8, 17, glass);
            }
            for i in 0..6 {
                let a0 = x0 + 0.15 + i as f32 * (x1 - x0 - 0.3) / 6.0;
                let a1 = a0 + (x1 - x0 - 0.3) / 6.0;
                let c = if i % 2 == 0 { palette::rgb(0xc8, 0x34, 0x30) } else { palette::rgb(0xf4, 0xf0, 0xe4) };
                let p = |a: f32, yy: f32, z: i32| cam.project(a, yy, base + z);
                fb.fill_poly(&[p(a0, body.3, 19), p(a1, body.3, 19), p(a1, body.3 + 0.3, 14), p(a0, body.3 + 0.3, 14)], c);
            }
            let fp = cam.project(x1 - 0.05, y1 - 0.9, base);
            let (fx, fy) = (fp.0 as i32, fp.1 as i32);
            for k in 0..44 {
                fb.put(fx, fy - k, palette::rgb(0xe0, 0xe0, 0xe4));
            }
            for i in 0..14 {
                let off = ((i as f32 * 0.7).sin() * 1.2).round() as i32;
                for j in 0..9 {
                    let c = if j % 3 == 1 { palette::rgb(0xf4, 0xf4, 0xf4) } else if i < 5 && j < 5 { palette::rgb(0x2c, 0x5c, 0xa8) } else { palette::rgb(0xc8, 0x34, 0x30) };
                    fb.put(fx + 1 + i, fy - 44 + j + off, c);
                }
            }
            cylinder(fb, cam, x0 + 0.4, y1 - 0.3, 4, base, 0, 12, palette::rgb(0xc8, 0x28, 0x2c));
            cylinder(fb, cam, x0 + 0.4, y1 - 0.3, 4, base, 12, 14, palette::rgb(0x8a, 0x1c, 0x20));
            let (vx, vy) = (x0 + 1.2, y1 - 0.55);
            iso_box(fb, cam, (vx, vy, vx + 0.85, vy + 0.34), base, 0, 9, palette::rgb(0xf4, 0xf0, 0xe4));
            iso_box(fb, cam, (vx, vy, vx + 0.85, vy + 0.34), base, 3, 5, palette::rgb(0xc8, 0x34, 0x30));
        }
    }
}
