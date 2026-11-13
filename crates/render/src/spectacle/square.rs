//! The square: stage, stalls, bunting and lanterns (B62).

use super::*;

pub(crate) const FEST_COLS: [Color; 5] = [
    palette::rgb(0xd8, 0x3a, 0x3a),
    palette::rgb(0xf0, 0xc8, 0x30),
    palette::rgb(0x2f, 0x6a, 0xc8),
    palette::rgb(0x2f, 0x9a, 0x5a),
    palette::rgb(0xf4, 0xf0, 0xe0),
];

/// A stall about 20 px wide and 17 tall: a striped awning on two poles over a
/// laden table. Bigger than the old 11-px stall, which read as a smudge at 1x.
pub fn stall(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, v: u8) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (bx, by) = cam.project(x, y, base);
    let (bx, by) = (bx as i32, by as i32);
    let cloth = FEST_COLS[v as usize % 4];
    let wood = palette::rgb(0x8a, 0x6a, 0x46);
    let post = palette::rgb(0x5a, 0x46, 0x32);
    fb.fill_rect(bx - 9, by - 15, 1, 12, post);
    fb.fill_rect(bx + 9, by - 15, 1, 12, post);
    fb.fill_rect(bx - 8, by - 6, 17, 2, palette::shade(wood, 0.8));
    fb.fill_rect(bx - 8, by - 4, 17, 3, wood);
    fb.fill_rect(bx - 8, by - 4, 17, 1, palette::shade(wood, 1.18));
    // Goods on the table, in colours that read: fruit, bread, flowers, jars.
    let goods = [
        pal::FLOWERS[0],
        palette::rgb(0xe8, 0xb0, 0x40),
        pal::LEAF[0],
        pal::FLOWERS[2],
        palette::rgb(0xd8, 0x70, 0x30),
    ];
    for i in 0..7 {
        let c = goods[(i + v as usize) % goods.len()];
        fb.put(bx - 7 + i as i32 * 2, by - 7, c);
        fb.put(bx - 7 + i as i32 * 2, by - 8, palette::shade(c, 0.85));
        fb.put(bx - 6 + i as i32 * 2, by - 7, palette::shade(c, 1.1));
    }
    // The awning: a striped slope with a scalloped front edge.
    for i in 0..21i32 {
        let stripe = if (i / 2) % 2 == 0 { cloth } else { palette::rgb(0xf4, 0xf0, 0xdc) };
        fb.put(bx - 10 + i, by - 17, palette::shade(stripe, 1.08));
        fb.put(bx - 10 + i, by - 16, stripe);
        fb.put(bx - 10 + i, by - 15, palette::shade(stripe, 0.88));
        if i % 2 == 0 {
            fb.put(bx - 10 + i, by - 14, palette::shade(stripe, 0.78));
        }
    }
    // The vendor behind the table: a head and shoulders.
    let coat = pal::PEOPLE_SHIRT[v as usize % pal::PEOPLE_SHIRT.len()];
    fb.fill_rect(bx - 1, by - 11, 3, 4, coat);
    fb.put(bx, by - 12, pal::PEOPLE_HEAD);
    fb.put(bx + 1, by - 12, pal::PEOPLE_HEAD);
}

/// A post for bunting or lanterns, `h` px tall.
pub fn post(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, h: i32) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let a = cam.project(x, y, base);
    let b = cam.project(x, y, base + h);
    fb.line(a, b, palette::rgb(0x4a, 0x3c, 0x30));
    fb.put(b.0 as i32, b.1 as i32 - 1, palette::rgb(0xd8, 0xc0, 0x78));
}

/// The stage: a wooden deck, a back wall with banners, a roof truss with its
/// lights, two speaker stacks and a drum kit. The rectangle is the deck on the
/// ground; it faces +y. Returns the deck's height in px (the band stands on it).
pub fn stage(fb: &mut Framebuffer, cam: &Camera, (x0, y0, x1, y1): (f32, f32, f32, f32), level: u8) -> i32 {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let deck_h = 8;
    let wood = palette::rgb(0x9a, 0x78, 0x52);
    let dark = palette::rgb(0x2a, 0x2c, 0x38);
    iso_box(fb, cam, (x0, y0, x1, y0 + 0.10), base, deck_h, deck_h + 24, palette::rgb(0x33, 0x36, 0x46));
    iso_box(fb, cam, (x0, y0, x1, y1), base, 0, deck_h, wood);
    for k in 1..8 {
        let f = k as f32 / 8.0;
        let a = cam.project(x0 + (x1 - x0) * f, y0, base + deck_h);
        let b = cam.project(x0 + (x1 - x0) * f, y1, base + deck_h);
        fb.line(a, b, palette::shade(wood, 0.84));
    }
    // Banners on the back wall: a run of coloured panels.
    let n = 5;
    for i in 0..n {
        let f0 = (i as f32 + 0.12) / n as f32;
        let f1 = (i as f32 + 0.88) / n as f32;
        let xa = x0 + (x1 - x0) * f0;
        let xb = x0 + (x1 - x0) * f1;
        let yb = y0 + 0.11;
        let c = FEST_COLS[i % 4];
        let q = [
            cam.project(xa, yb, base + deck_h + 5),
            cam.project(xb, yb, base + deck_h + 5),
            cam.project(xb, yb, base + deck_h + 19),
            cam.project(xa, yb, base + deck_h + 19),
        ];
        fb.fill_poly(&q, c);
        fb.line(q[0], q[1], palette::shade(c, 1.25));
    }
    // Side towers and the roof truss with its lamps.
    for xx in [x0 - 0.06, x1 - 0.04] {
        iso_box(fb, cam, (xx, y0, xx + 0.10, y0 + 0.14), base, deck_h, deck_h + 34, dark);
    }
    iso_box(fb, cam, (x0 - 0.08, y0, x1 + 0.08, y0 + 0.52), base, deck_h + 31, deck_h + 35, palette::shade(dark, 1.25));
    for i in 0..7 {
        let f = (i as f32 + 0.5) / 7.0;
        let p = cam.project(x0 + (x1 - x0) * f, y0 + 0.5, base + deck_h + 30);
        let c = [pal::WINDOW_LIT, palette::rgb(0xff, 0x90, 0xd0), palette::rgb(0x90, 0xe0, 0xff)][i % 3];
        fb.put(p.0 as i32, p.1 as i32, c);
        fb.put(p.0 as i32, p.1 as i32 + 1, palette::shade(c, 0.8));
    }
    // Speaker stacks at the front corners, a drum kit at the back.
    for xx in [x0 + 0.10, x1 - 0.42] {
        iso_box(fb, cam, (xx, y1 - 0.42, xx + 0.30, y1 - 0.14), base, deck_h, deck_h + 13, dark);
        let c = cam.project(xx + 0.15, y1 - 0.14, base + deck_h + 5);
        fb.put(c.0 as i32, c.1 as i32, palette::rgb(0x70, 0x74, 0x88));
        fb.put(c.0 as i32 + 1, c.1 as i32 + 1, palette::rgb(0x70, 0x74, 0x88));
    }
    let dk = cam.project((x0 + x1) * 0.5 + 0.35, y0 + 0.42, base + deck_h);
    let (dx, dy) = (dk.0 as i32, dk.1 as i32);
    fb.fill_rect(dx - 4, dy - 4, 9, 3, palette::rgb(0xb8, 0x30, 0x30));
    fb.fill_rect(dx - 4, dy - 4, 9, 1, palette::rgb(0xe8, 0xe8, 0xf0));
    fb.fill_rect(dx - 7, dy - 7, 3, 1, palette::rgb(0xd8, 0xb0, 0x40));
    fb.fill_rect(dx + 5, dy - 7, 3, 1, palette::rgb(0xd8, 0xb0, 0x40));
    deck_h
}

/// The plaza's festival geometry in tile coordinates, shared by the static art
/// and the dynamic parts (crowd, lanterns) so they always agree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Square {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub level: u8,
}

impl Square {
    pub fn stage_rect(&self) -> (f32, f32, f32, f32) {
        // Off to the right of the middle, clear of the clock tower in the
        // plaza's far corner.
        let cx = self.x + self.w * 0.60;
        let half = (self.w * 0.25).clamp(0.9, 1.25);
        (cx - half, self.y + 0.22, cx + half, self.y + 1.18)
    }

    fn rows(&self) -> i32 {
        ((self.h - 1.6) / 1.15).floor().max(1.0) as i32
    }

    /// Stall spots along the two long sides, clear of the stage and fountain.
    pub fn stall_spots(&self, night: bool) -> Vec<(f32, f32)> {
        let mut v = Vec::new();
        for i in 0..self.rows() {
            let yy = self.y + 1.65 + i as f32 * 1.15;
            if yy > self.y + self.h - 0.45 {
                break;
            }
            v.push((self.x + 0.62, yy));
            v.push((self.x + self.w - 0.62, yy));
        }
        if night {
            let n = (self.w / 1.4) as i32;
            for i in 0..n {
                v.push((self.x + 1.4 + i as f32 * 1.2, self.y + self.h - 0.5));
            }
        }
        v
    }

    /// Posts round the edge, from which the bunting hangs, grouped by side
    /// (back, front, left, right), each ordered along its side.
    pub fn sides(&self) -> [Vec<(f32, f32)>; 4] {
        let (x0, y0, x1, y1) = (self.x + 0.2, self.y + 0.2, self.x + self.w - 0.2, self.y + self.h - 0.2);
        let nx = (self.w / 2.0).ceil().max(2.0) as i32;
        let ny = (self.h / 2.0).ceil().max(2.0) as i32;
        let mut back = Vec::new();
        let mut front = Vec::new();
        let mut left = Vec::new();
        let mut right = Vec::new();
        for i in 0..=nx {
            let f = i as f32 / nx as f32;
            back.push((x0 + (x1 - x0) * f, y0));
            front.push((x0 + (x1 - x0) * f, y1));
        }
        for i in 0..=ny {
            let f = i as f32 / ny as f32;
            left.push((x0, y0 + (y1 - y0) * f));
            right.push((x1, y0 + (y1 - y0) * f));
        }
        [back, front, left, right]
    }

    /// Lantern strings for the night market, hung across the aisles.
    pub fn lantern_lines(&self) -> Vec<(f32, f32, f32, f32)> {
        let mut v = Vec::new();
        for i in 0..self.rows() {
            let yy = self.y + 1.65 + i as f32 * 1.15 + 0.4;
            if yy > self.y + self.h - 0.45 {
                break;
            }
            v.push((self.x + 0.55, yy, self.x + self.w - 0.55, yy));
        }
        v
    }

    /// A walkable aisle down the middle of the square, for strollers.
    pub fn aisle(&self) -> ((f32, f32), (f32, f32)) {
        let cx = self.x + self.w * 0.5;
        ((cx - 0.9, self.y + 1.5), (cx - 0.9, self.y + self.h - 0.5))
    }
}

/// The plaza dressed for a festival and/or the night market (static layer).
pub fn festival_static(fb: &mut Framebuffer, cam: &Camera, sq: &Square, dress: Dress) {
    let lvl = sq.level;
    let base = lvl as i32 * crate::iso::HEIGHT_UNIT;
    let sides = sq.sides();
    for side in &sides {
        for &(px, py) in side {
            post(fb, cam, px, py, lvl, 26);
        }
    }
    if dress.festival {
        for side in &sides {
            for w in side.windows(2) {
                bunting(fb, cam, (w[0].0, w[0].1, base + 26), (w[1].0, w[1].1, base + 26), &FEST_COLS, 2.5);
            }
        }
        stage(fb, cam, sq.stage_rect(), lvl);
    }
    for (i, &(x, y)) in sq.stall_spots(dress.night_market && !dress.festival).iter().enumerate() {
        stall(fb, cam, x, y, lvl, i as u8);
    }
    if dress.night_market {
        let c = [palette::rgb(0x5a, 0x46, 0x32)];
        for (x0, y0, x1, y1) in sq.lantern_lines() {
            bunting(fb, cam, (x0, y0, base + 22), (x1, y1, base + 22), &c, 3.0);
        }
    }
}

/// The lanterns of the night market, drawn per frame so they glow after dark.
/// `lit` is whether it is dark enough for them to be lit.
pub fn night_lights(fb: &mut Framebuffer, cam: &Camera, sq: &Square, dress: Dress, lit: bool) {
    if !dress.night_market {
        return;
    }
    let base = sq.level as i32 * crate::iso::HEIGHT_UNIT;
    let cols = [
        palette::rgb(0xff, 0x5a, 0x3a),
        palette::rgb(0xff, 0xb0, 0x40),
        palette::rgb(0xff, 0x80, 0x50),
        palette::rgb(0xff, 0xd8, 0x60),
    ];
    for (li, (x0, y0, x1, y1)) in sq.lantern_lines().into_iter().enumerate() {
        let a = cam.project(x0, y0, base + 22);
        let b = cam.project(x1, y1, base + 22);
        let n = (((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() / 9.0).max(3.0) as i32;
        for i in 1..n {
            let f = i as f32 / n as f32;
            let x = a.0 + (b.0 - a.0) * f;
            let y = a.1 + (b.1 - a.1) * f + f * (1.0 - f) * 4.0 * 3.0 + 3.0;
            lantern(fb, x as i32, y as i32, cols[(i as usize + li) % cols.len()], lit);
        }
    }
}

/// The square dressed for a home game (B53/B62): blue-white-red bunting round
/// the edge and a big banner across the front, in the two team colours. Plain
/// colours and stripes only; no marks of any club.
pub fn gameday_static(fb: &mut Framebuffer, cam: &Camera, sq: &Square) {
    banner_dress(fb, cam, sq, [super::places::BLUE, super::places::RED], false);
}

/// The square dressed with a banner and bunting in two colours (B53/B62): the
/// home-game blue and red, or a personal date's own colours; a personal one
/// also ties a cluster of balloons to each banner pole.
pub fn banner_dress(fb: &mut Framebuffer, cam: &Camera, sq: &Square, two: [Color; 2], balloons: bool) {
    let lvl = sq.level;
    let base = lvl as i32 * crate::iso::HEIGHT_UNIT;
    let cols = [two[0], palette::rgb(0xf4, 0xf4, 0xf4), two[1]];
    let sides = sq.sides();
    for side in &sides {
        for &(px, py) in side {
            post(fb, cam, px, py, lvl, 30);
        }
        for w in side.windows(2) {
            bunting(fb, cam, (w[0].0, w[0].1, base + 30), (w[1].0, w[1].1, base + 30), &cols, 3.0);
        }
    }
    // The banner: a 3.2-tile strip between two tall poles, striped blue and red
    // with white edges, the way a team's banner hangs over a high street.
    let (cx, cy) = (sq.x + sq.w * 0.5, sq.y + sq.h - 0.25);
    let (hx0, hx1) = (cx - 1.7, cx + 1.7);
    let a = cam.project(hx0, cy, base);
    let b = cam.project(hx1, cy, base);
    let top = base + 56;
    for (px, py) in [a, b] {
        fb.line((px, py), (px, py - 56.0), palette::rgb(0xd8, 0xd4, 0xc8));
        fb.put(px as i32, py as i32 - 57, palette::rgb(0xf0, 0xd0, 0x40));
    }
    // The cloth: bands as polygons, so the diagonal banner is a solid surface
    // (a column-by-column fill read as a picket fence).
    let at = |f: f32, z: i32| {
        let q = cam.project(hx0 + (hx1 - hx0) * f, cy, base + z);
        (q.0, q.1 + f * (1.0 - f) * 12.0)
    };
    let (z_top, z_bot) = (56, 42);
    let blocks = 8;
    for j in 0..blocks {
        let (f0, f1) = (j as f32 / blocks as f32, (j + 1) as f32 / blocks as f32);
        let c = if j % 2 == 0 { two[0] } else { two[1] };
        let q = [at(f0, z_top - 2), at(f1, z_top - 2), at(f1, z_bot + 2), at(f0, z_bot + 2)];
        fb.fill_poly(&q, c);
    }
    if balloons {
        for (px, py) in [a, b] {
            for (i, (dx, dy)) in [(-6.0f32, -76.0f32), (2.0, -82.0), (8.0, -74.0)].iter().enumerate() {
                let c = [two[0], two[1], palette::rgb(0xf0, 0xd0, 0x40)][i];
                fb.line((px, py - 58.0), (px + dx, py + dy + 4.0), palette::rgb(0xe8, 0xe4, 0xd8));
                crate::tex::ellipse(fb, px + dx, py + dy, 4.0, 5.0, c);
                fb.put((px + dx) as i32 - 1, (py + dy) as i32 - 2, palette::mix(c, palette::rgb(0xff, 0xff, 0xff), 0.7));
            }
        }
    }
    // White edges top and bottom.
    for (za, zb) in [(z_top, z_top - 2), (z_bot + 2, z_bot)] {
        let q = [at(0.0, za), at(1.0, za), at(1.0, zb), at(0.0, zb)];
        fb.fill_poly(&q, palette::rgb(0xf4, 0xf4, 0xf4));
    }
}
