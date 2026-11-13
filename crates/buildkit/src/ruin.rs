//! What the world's endings do to a building (B50): a post-pass over a
//! finished kit sprite, never a second catalogue. The same building, hit,
//! burnt, collapsed, grown over, converted by machines, bleached, shut up,
//! frosted, soot-stained or just dark.
//!
//! The codes are `sim::ending::R_*`:
//! 1 fire, 2 rubble, 3 overgrown, 4 machine, 5 bleach, 6 abandoned, 7 frost,
//! 8 soot, 10 dark machine, 11 dark (lights out), and B52's two civic states:
//! 12 vacant (lights out, boards on the ground floor, a "for lease" banner) and
//! 13 scorched (a fire's black plumes over the windows). 0 and 9 never get here.
//!
//! Pixels are `0xAARRGGBB`. Alpha `0xFE` is *emissive* (the renderer does not
//! tint it at night), which is how embers, flames and the machine lights glow.

const EMBER: u32 = 0xFE00_0000;

fn hash(seed: u32, x: i32, y: i32) -> f32 {
    let mut h = seed
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((x as u32).wrapping_mul(0x85EB_CA6B))
        .wrapping_add((y as u32).wrapping_mul(0xC2B2_AE35));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65536.0
}

/// Smooth 1-D value noise along a row, `step` px per lattice point.
fn vnoise(seed: u32, x: i32, step: i32) -> f32 {
    let i = x.div_euclid(step);
    let f = x.rem_euclid(step) as f32 / step as f32;
    let a = hash(seed, i, 7);
    let b = hash(seed, i + 1, 7);
    let f = f * f * (3.0 - 2.0 * f);
    a + (b - a) * f
}

fn rgb(c: u32) -> (f32, f32, f32) {
    (((c >> 16) & 0xFF) as f32, ((c >> 8) & 0xFF) as f32, (c & 0xFF) as f32)
}

fn pack(r: f32, g: f32, b: f32, a: u32) -> u32 {
    (a << 24)
        | ((r.clamp(0.0, 255.0) as u32) << 16)
        | ((g.clamp(0.0, 255.0) as u32) << 8)
        | (b.clamp(0.0, 255.0) as u32)
}

fn mix(c: u32, t: (u8, u8, u8), k: f32) -> u32 {
    let (r, g, b) = rgb(c);
    pack(
        r + (t.0 as f32 - r) * k,
        g + (t.1 as f32 - g) * k,
        b + (t.2 as f32 - b) * k,
        c >> 24,
    )
}

fn shade(c: u32, k: f32) -> u32 {
    let (r, g, b) = rgb(c);
    pack(r * k, g * k, b * k, c >> 24)
}

fn lum(c: u32) -> f32 {
    let (r, g, b) = rgb(c);
    (0.3 * r + 0.59 * g + 0.11 * b) / 255.0
}

struct Img<'a> {
    px: &'a mut Vec<u32>,
    w: usize,
    h: usize,
}

impl Img<'_> {
    fn at(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0
        } else {
            self.px[y as usize * self.w + x as usize]
        }
    }
    fn set(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = c;
        }
    }
    fn solid(&self, x: i32, y: i32) -> bool {
        (self.at(x, y) >> 24) >= 0x80
    }
    /// The top-most and bottom-most opaque row of a column.
    fn span(&self, x: i32) -> Option<(i32, i32)> {
        let mut top = None;
        let mut bot = 0;
        for y in 0..self.h as i32 {
            if self.solid(x, y) {
                if top.is_none() {
                    top = Some(y);
                }
                bot = y;
            }
        }
        top.map(|t| (t, bot))
    }
    fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                if self.solid(x, y) {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        if x1 < x0 {
            None
        } else {
            Some((x0, y0, x1, y1))
        }
    }
}

/// Switch off every lit pane: emissive pixels become dark glass.
fn lights_out(im: &mut Img) {
    for p in im.px.iter_mut() {
        if (*p >> 24) == 0xFE {
            *p = 0xFF1A_2232;
        }
    }
}

/// Apply ruin `code` to a sprite in place. `seed` varies the damage per
/// building; `(ox, oy)` is the sprite anchor (unused by most passes).
pub fn apply(px: &mut Vec<u32>, w: usize, h: usize, _ox: i32, _oy: i32, code: u8, seed: u32) {
    let mut im = Img { px, w, h };
    match code {
        1 => fire(&mut im, seed),
        2 => rubble(&mut im, seed, false),
        3 => rubble(&mut im, seed, true),
        4 => machine(&mut im, seed, true),
        10 => machine(&mut im, seed, false),
        5 => bleach(&mut im, seed),
        6 => abandoned(&mut im, seed),
        7 => frost(&mut im, seed),
        8 => soot(&mut im, seed),
        11 => lights_out(&mut im),
        12 => vacant(&mut im, seed),
        13 => scorched(&mut im, seed),
        _ => {}
    }
}

// --- fire: a roofline bitten out, scorched, burning ---------------------

fn fire(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    let height = (y1 - y0).max(1);
    // The ragged bite out of the top: deep in one place, shallow in another.
    let bite_at = hash(seed, 3, 3) * (x1 - x0) as f32 + x0 as f32;
    let bite_w = 8.0 + hash(seed, 4, 4) * (x1 - x0).max(8) as f32 * 0.35;
    for x in x0..=x1 {
        let Some((top, bot)) = im.span(x) else { continue };
        let near = (1.0 - ((x as f32 - bite_at).abs() / bite_w)).max(0.0);
        let n = vnoise(seed, x, 3);
        let depth = ((0.10 + 0.34 * near + 0.14 * n) * height as f32).min((bot - top) as f32 * 0.7) as i32;
        for y in top..top + depth {
            im.set(x, y, 0);
        }
        // Exposed floors and char just under the cut.
        for k in 0..3 {
            let y = top + depth + k;
            if im.solid(x, y) {
                let c = if k == 0 { 0xFF1B_1411 } else { 0xFF2A_2019 };
                if hash(seed, x, y) < 0.85 {
                    im.set(x, y, c);
                }
            }
        }
    }
    // Soot blooms.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let n = vnoise(seed ^ 0x55, x, 5) * 0.6 + vnoise(seed ^ 0x77, y, 4) * 0.4;
            let top_bias = 1.0 - (y - y0) as f32 / height as f32;
            if n + top_bias * 0.35 > 0.78 {
                im.set(x, y, shade(mix(c, (0x20, 0x1a, 0x18), 0.5), 0.8));
            }
        }
    }
    // Flames along the cut, and a few windows spitting sparks.
    for x in x0..=x1 {
        let Some((top, _)) = im.span(x) else { continue };
        if hash(seed ^ 0xF1, x, 1) < 0.34 {
            let hgt = 3 + (hash(seed ^ 0xF2, x, 2) * 7.0) as i32;
            for k in 0..hgt {
                let y = top - 1 - k;
                let t = k as f32 / hgt as f32;
                let col = if t < 0.3 {
                    0xFEFF_F0A0
                } else if t < 0.6 {
                    0xFEFF_B02A
                } else if t < 0.85 {
                    0xFEFF_6A1E
                } else {
                    0xFEC8_3414
                };
                if hash(seed ^ 0xF3, x, k) < 0.92 - t * 0.2 {
                    im.set(x, y, col);
                    if hgt > 6 && k < hgt / 2 && hash(seed ^ 0xF4, x + 1, k) < 0.5 {
                        im.set(x + 1, y, col);
                    }
                }
            }
        }
    }
    // Embers in the char.
    for y in y0..=y1 {
        for x in x0..=x1 {
            if im.solid(x, y) && im.at(x, y) & 0x00FF_FFFF == 0x001B_1411 && hash(seed ^ 0xE1, x, y) < 0.3 {
                im.set(x, y, EMBER | 0x00FF_5A1A);
            }
        }
    }
}

// --- rubble: a heap, a stub of wall, beams ------------------------------

fn rubble(im: &mut Img, seed: u32, grown: bool) {
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    let height = (y1 - y0).max(1);
    // Colours the building was made of.
    let mut cols: Vec<u32> = Vec::new();
    for y in (y0..=y1).step_by(2) {
        for x in (x0..=x1).step_by(2) {
            let c = im.at(x, y);
            if c >> 24 >= 0x80 && (c >> 24) != 0xFE {
                cols.push(c | 0xFF00_0000);
            }
        }
    }
    if cols.is_empty() {
        return;
    }
    let mut out = vec![0u32; im.w * im.h];
    let cx = (x0 + x1) as f32 * 0.5;
    let half = ((x1 - x0) as f32 * 0.5).max(1.0);
    let peak = ((height as f32 * 0.22).min(22.0)).max(6.0);
    let keep = (height as f32 * (0.10 + 0.08 * hash(seed, 9, 9))) as i32 + 3;
    let dust = (0x6e, 0x66, 0x5e);
    let put = |out: &mut Vec<u32>, w: usize, h: usize, x: i32, y: i32, c: u32| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            out[y as usize * w + x as usize] = c;
        }
    };
    for x in x0..=x1 {
        let Some((_, bot)) = im.span(x) else { continue };
        let u = ((x as f32 - cx) / half).abs();
        // A mound with two lumps, its toe a little ragged.
        let bell = (1.0 - u.powf(1.7)).max(0.0).powf(0.85);
        let lumpy = 0.62 + 0.38 * vnoise(seed, x, 6);
        let hh = (peak * bell * lumpy).round() as i32 + 2;
        // The stub of the old wall, ragged along its top.
        let stub = ((keep as f32) * (0.35 + 0.65 * vnoise(seed ^ 0x31, x, 3))) as i32;
        for y in (bot - stub)..=bot {
            let c = im.at(x, y);
            if c >> 24 >= 0x80 && hash(seed ^ 0x32, x, y) > 0.12 {
                let c = mix(shade(c | 0xFF00_0000, 0.72), dust, 0.22);
                put(&mut out, im.w, im.h, x, y, c | 0xFF00_0000);
            }
        }
        // The heap: blocky chunks (3 x 2 px) of the building's own colours.
        for y in (bot - hh)..=bot {
            let depth = bot - y;
            let (cxk, cyk) = (x.div_euclid(3), y.div_euclid(2));
            let pick = (hash(seed ^ 0xB1, cxk, cyk) * cols.len() as f32) as usize;
            let mut c = cols[pick.min(cols.len() - 1)];
            let tone = 0.5 + 0.36 * hash(seed ^ 0xB2, cxk, cyk);
            c = shade(mix(c, dust, 0.28), tone);
            // Lit from the top, dark at the toe and at the left.
            if depth >= hh - 1 {
                c = shade(c, 1.35);
            } else if depth <= 1 {
                c = shade(c, 0.62);
            }
            if hash(seed ^ 0xB3, x, y) > 0.965 {
                c = 0xFFE0_D8C4; // a pale chip of plaster or glass
            }
            put(&mut out, im.w, im.h, x, y, c | 0xFF00_0000);
        }
    }
    // Beams and bent rebar laid across the heap.
    for k in 0..5 {
        let bx = x0 + 3 + (hash(seed ^ 0xB4, k, 1) * (x1 - x0 - 6).max(1) as f32) as i32;
        let mut by = y1;
        for y in (y0..=y1).rev() {
            if out[y as usize * im.w + bx as usize] >> 24 >= 0x80 {
                by = y;
            }
        }
        let len = 5 + (hash(seed ^ 0xB5, k, 2) * 9.0) as i32;
        let sl = (hash(seed ^ 0xB6, k, 3) - 0.5) * 1.4;
        let colr = if k % 2 == 0 { 0xFF2A_2420 } else { 0xFFB4_A488 };
        for t in 0..len {
            put(&mut out, im.w, im.h, bx + (t as f32 * sl.signum()) as i32, by - 1 - (t as f32 * sl.abs() * 1.6) as i32, colr);
        }
    }
    if grown {
        greenery(&mut out, im.w, im.h, (x0, y0, x1, y1), seed);
    }
    im.px.copy_from_slice(&out);
}

/// Vines, saplings and grass over a heap or a wall.
fn greenery(out: &mut [u32], w: usize, h: usize, b: (i32, i32, i32, i32), seed: u32) {
    let (x0, _y0, x1, y1) = b;
    let greens = [0xFF2F_6B2E, 0xFF3F_8A3A, 0xFF58_A84A, 0xFF7B_C45C];
    let put = |out: &mut [u32], x: i32, y: i32, c: u32| {
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            out[y as usize * w + x as usize] = c;
        }
    };
    // Vines creep over what is there.
    for y in 0..h as i32 {
        for x in x0..=x1 {
            let i = y as usize * w + x as usize;
            if out[i] >> 24 < 0x80 {
                continue;
            }
            let n = vnoise(seed ^ 0xC1, x, 4) * 0.5 + vnoise(seed ^ 0xC2, y, 3) * 0.5;
            if n > 0.46 && hash(seed ^ 0xC3, x, y) < 0.78 {
                let g = greens[((hash(seed ^ 0xC4, x, y) * 3.0) as usize).min(3)];
                out[i] = g;
            }
        }
    }
    // Saplings standing up out of the heap.
    for k in 0..3 {
        let x = x0 + 4 + (hash(seed ^ 0xC5, k, 1) * (x1 - x0 - 8).max(1) as f32) as i32;
        let mut ytop = y1;
        for y in 0..h as i32 {
            if out[y as usize * w + x.max(0) as usize] >> 24 >= 0x80 {
                ytop = y;
                break;
            }
        }
        let th = 4 + (hash(seed ^ 0xC6, k, 2) * 6.0) as i32;
        for t in 0..th {
            put(out, x, ytop - t, 0xFF5A_4430);
        }
        let r = 2 + (hash(seed ^ 0xC7, k, 3) * 2.0) as i32;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r + 1 && hash(seed ^ 0xC8, x + dx, ytop + dy) < 0.88 {
                    let g = greens[((hash(seed ^ 0xC9, x + dx, ytop + dy) * 4.0) as usize).min(3)];
                    put(out, x + dx, ytop - th + dy, g);
                }
            }
        }
    }
    // Grass at the foot.
    for x in x0..=x1 {
        if hash(seed ^ 0xCA, x, 3) < 0.5 {
            let n = 1 + (hash(seed ^ 0xCB, x, 4) * 3.0) as i32;
            for t in 0..n {
                put(out, x, y1 - t, greens[((hash(seed ^ 0xCC, x, t) * 4.0) as usize).min(3)]);
            }
        }
    }
}

// --- machine: one uniform panelled style, lights in slits ---------------

fn machine(im: &mut Img, seed: u32, lit: bool) {
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    let height = (y1 - y0).max(1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let l = lum(c);
            // Brushed blue-grey, from the old shading.
            let t = (l * 0.9).clamp(0.0, 1.0);
            let base = (
                0x22 as f32 + (0x86 - 0x22) as f32 * t,
                0x2c as f32 + (0x98 - 0x2c) as f32 * t,
                0x3a as f32 + (0xb2 - 0x3a) as f32 * t,
            );
            let mut p = pack(base.0, base.1, base.2, 0xFF);
            // Panel seams.
            if (y - y0) % 7 == 0 {
                p = shade(p, 0.62);
            } else if (x - x0) % 11 == 0 {
                p = shade(p, 0.78);
            }
            // The roof plate: darker, with a fine grid.
            let from_top = (y - y0) as f32 / height as f32;
            if from_top < 0.06 {
                p = shade(p, 0.8);
            }
            im.set(x, y, p);
        }
    }
    // Light slits: regular rows, cyan, on every wall.
    for y in y0..=y1 {
        for x in x0..=x1 {
            if !im.solid(x, y) {
                continue;
            }
            let row = (y - y0) % 9;
            let col = (x - x0) % 6;
            if row == 4 && col < 4 && (y - y0) as f32 > height as f32 * 0.1 {
                if lit {
                    im.set(x, y, 0xFE38_E0FF);
                } else if hash(seed, x / 6, y / 9) < 0.15 {
                    im.set(x, y, 0xFF14_3C48);
                } else {
                    im.set(x, y, 0xFF16_1E28);
                }
            }
        }
    }
    // An antenna with a cold light.
    if let Some((top, _)) = im.span((x0 + x1) / 2) {
        for k in 1..=6 {
            im.set((x0 + x1) / 2, top - k, 0xFF3A_4658);
        }
        im.set((x0 + x1) / 2, top - 7, if lit { 0xFEFF_4A4A } else { 0xFF50_2A2A });
    }
    if !lit {
        for p in im.px.iter_mut() {
            if *p >> 24 >= 0x80 {
                *p = shade(*p, 0.72) | 0xFF00_0000;
            }
        }
    }
}

// --- bleach: the sun has had a long look at it --------------------------

fn bleach(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let mut p = mix(c, (0xee, 0xe2, 0xcc), 0.52);
            // Cracked and chalky.
            if hash(seed, x, y) > 0.965 {
                p = shade(p, 0.78);
            }
            im.set(x, y, p | 0xFF00_0000);
        }
    }
}

// --- abandoned: boards, weeds, no lights --------------------------------

fn abandoned(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    // Weathered.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let mut p = mix(shade(c, 0.8), (0x72, 0x6c, 0x60), 0.38);
            // Rain streaks.
            if vnoise(seed, x, 2) > 0.7 && hash(seed ^ 0xA1, x, y / 6) < 0.5 {
                p = shade(p, 0.82);
            }
            im.set(x, y, p | 0xFF00_0000);
        }
    }
    // Boards over the windows: small brown planks on the wall.
    let n = ((x1 - x0) * (y1 - y0)) / 55;
    for k in 0..n {
        let x = x0 + (hash(seed ^ 0xA2, k, 1) * (x1 - x0) as f32) as i32;
        let y = y0 + (hash(seed ^ 0xA3, k, 2) * (y1 - y0) as f32) as i32;
        if (y - y0) % 7 > 1 || !im.solid(x, y) || !im.solid(x + 3, y + 3) {
            continue;
        }
        for dy in 0..4 {
            for dx in 0..4 {
                if im.solid(x + dx, y + dy) {
                    let plank = if dy % 2 == 0 { 0xFF6B_5236 } else { 0xFF58_4129 };
                    im.set(x + dx, y + dy, plank);
                }
            }
        }
    }
    // Weeds along the foot and ivy up one corner.
    let greens = [0xFF3F_8A3A, 0xFF58_A84A, 0xFF2F_6B2E];
    for x in x0..=x1 {
        if let Some((_, bot)) = im.span(x) {
            if hash(seed ^ 0xA4, x, 1) < 0.45 {
                let hgt = 1 + (hash(seed ^ 0xA5, x, 2) * 3.0) as i32;
                for t in 0..hgt {
                    im.set(x, bot - t, greens[(x as usize + t as usize) % 3]);
                }
            }
        }
    }
    let ix = if hash(seed ^ 0xA6, 5, 5) < 0.5 { x0 + 2 } else { x1 - 3 };
    for y in (y0 + (y1 - y0) / 3)..=y1 {
        for dx in 0..3 {
            if im.solid(ix + dx, y) && vnoise(seed ^ 0xA7, y, 3) > 0.38 + dx as f32 * 0.12 {
                im.set(ix + dx, y, greens[(y as usize + dx as usize) % 3]);
            }
        }
    }
}

// --- frost: white roofs, blue shade, icicles ----------------------------

fn frost(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, _y0, x1, _y1)) = im.bounds() else { return };
    for x in x0..=x1 {
        let Some((top, bot)) = im.span(x) else { continue };
        // The roof: the top of the column, deep enough to cover the roof plane.
        // B99c: a cap of five to ten pixels only painted the ridge, so every
        // building wore a white **outline** and the snow itself read as specks
        // of noise. The roof's depth on screen follows the building's width (a
        // rhombus half as tall as it is wide), so the cap is a share of the
        // sprite's width, never more than the column allows.
        let col = (bot - top).max(1) as f32;
        let wide = (x1 - x0).max(4) as f32;
        let cap = (0.30 * wide).clamp(4.0, 15.0) + vnoise(seed, x, 4) * 2.5 - 1.0;
        let cap = cap.min(col).max(1.0);
        for y in top..=bot {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let d = (y - top) as f32;
            let p = if d < cap {
                // Snow on the roof: bright on top, blue-grey in its own shade,
                // and a dithered line where it thins out over the eave.
                let k = d / cap;
                if k > 0.86 {
                    let r = hash(seed ^ 0x7A, x, y);
                    if r < 0.45 {
                        0xFFE4_EEF8
                    } else if r < 0.8 {
                        0xFFC9_DDEF
                    } else {
                        shade(mix(c, (0xb8, 0xd2, 0xee), 0.30), 0.98)
                    }
                } else if k > 0.6 {
                    let t = (k - 0.6) / 0.26;
                    let (r, g, b) = rgb(0xFFF3_F8FF);
                    let (r2, g2, b2) = rgb(0xFFCF_E2F3);
                    pack(
                        r + (r2 - r) * t,
                        g + (g2 - g) * t,
                        b + (b2 - b) * t,
                        0xFF,
                    )
                } else {
                    0xFFF7_FBFF
                }
            } else if d < cap + 2.0 {
                // The eave: a shallow shadow under the snow line.
                shade(mix(c, (0x9f, 0xbc, 0xd8), 0.30), 0.94)
            } else {
                shade(mix(c, (0xb8, 0xd2, 0xee), 0.24), 0.96)
            };
            im.set(x, y, p | 0xFF00_0000);
        }
        // Icicles under the snow line, from the eave.
        if hash(seed ^ 0x1C, x, 1) < 0.16 {
            let len = 2 + (hash(seed ^ 0x1D, x, 2) * 4.0) as i32;
            for k in 0..len {
                let y = top + cap as i32 + 2 + k;
                if im.solid(x, y) {
                    im.set(x, y, if k == len - 1 { 0xFFDD_EEFF } else { 0xFFB8_D8F2 });
                }
            }
        }
    }
}

// --- soot: smoke and ash ------------------------------------------------

fn soot(im: &mut Img, seed: u32) {
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let streak = if vnoise(seed, x, 2) > 0.62 && hash(seed ^ 0x51, x, y / 5) < 0.6 { 0.8 } else { 1.0 };
            let alpha = c >> 24;
            let mut p = shade(mix(c, (0x4a, 0x40, 0x38), 0.32), 0.72 * streak);
            if alpha == 0xFE {
                p = (p & 0x00FF_FFFF) | (0xFE << 24);
            } else {
                p |= 0xFF00_0000;
            }
            im.set(x, y, p);
        }
    }
}

// --- B52: vacant and scorched -----------------------------------------------

/// A building nobody rents: the lights are out, the street-level windows are
/// boarded, and a "for lease" banner hangs on the front. Not derelict (no weeds,
/// no rubble): it was open last month.
fn vacant(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    let (w, h) = (x1 - x0, y1 - y0);
    // A little dust on everything.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 >= 0x80 {
                im.set(x, y, mix(c, (0x70, 0x6c, 0x66), 0.22) | (c & 0xFF00_0000));
            }
        }
    }
    // Boards over the ground-floor panes: any dark pixel in the lowest fifth of
    // the building gets a plank colour (dark glass is the only dark thing there).
    let floor_top = y1 - (h / 5).max(6);
    for y in floor_top..=y1 - 2 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 >= 0x80 && lum(c) < 0.2 {
                let plank = if (y - floor_top) % 3 == 0 { 0xFF58_4129 } else { 0xFF72_5A3C };
                im.set(x, y, plank);
            }
        }
    }
    // The banner, on the front face: white cloth, a red band, dark lettering.
    let bw = (w / 2).clamp(20, 34);
    let bx = x0 + (w as f32 * (0.50 + 0.18 * hash(seed, 4, 4))) as i32;
    let by = y0 + (h as f32 * 0.50) as i32;
    let bx = bx.clamp(x0 + 2, (x1 - bw - 2).max(x0 + 2));
    if im.solid(bx, by) && im.solid(bx + bw - 1, by + 9) {
        for dy in 0..10 {
            for dx in 0..bw {
                let c = if dy == 0 || dy == 9 || dx == 0 || dx == bw - 1 {
                    0xFF20_1C1C
                } else if dy == 1 || dy == 8 {
                    0xFFC8_3C32
                } else if (dy == 3 || dy == 4) && dx % 4 != 3 && dx > 2 && dx < bw - 3 {
                    0xFF40_3C3C
                } else if (dy == 6) && dx % 2 == 0 && dx > 2 && dx < bw - 3 {
                    0xFF40_3C3C
                } else {
                    0xFFEE_EAE0
                };
                im.set(bx + dx, by + dy, c);
            }
        }
        // Two ropes.
        im.set(bx, by - 1, 0xFF30_2C28);
        im.set(bx + bw - 1, by - 1, 0xFF30_2C28);
    }
}

/// The wall after a fire: black plumes streaking up from every window and a
/// sooty tide over the whole front. The roof is whole (the damage is inside).
fn scorched(im: &mut Img, seed: u32) {
    lights_out(im);
    let Some((x0, y0, x1, y1)) = im.bounds() else { return };
    // Darken the whole building towards charcoal.
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 < 0x80 {
                continue;
            }
            let k = 0.28 + 0.2 * hash(seed ^ 0x5C, x / 3, y / 3);
            im.set(x, y, mix(shade(c, 0.78), (0x1a, 0x16, 0x14), k) | (c & 0xFF00_0000));
        }
    }
    // Plumes above every dark pane.
    for y in y0 + 3..=y1 {
        for x in x0..=x1 {
            let c = im.at(x, y);
            if c >> 24 >= 0x80 && lum(c) < 0.12 && hash(seed ^ 0x77, x, y) < 0.8 {
                let len = 3 + (hash(seed ^ 0x78, x, y) * 6.0) as i32;
                for t in 1..=len {
                    let (px, py) = (x + (t / 4) * if hash(seed, x, y) < 0.5 { 1 } else { -1 }, y - t);
                    let d = im.at(px, py);
                    if d >> 24 >= 0x80 {
                        im.set(px, py, mix(d, (0x0c, 0x0a, 0x0a), 0.7 - t as f32 * 0.07) | (d & 0xFF00_0000));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> (Vec<u32>, usize, usize) {
        let (w, h) = (48usize, 60usize);
        let mut px = vec![0u32; w * h];
        for y in 10..58 {
            for x in 6..42 {
                px[y * w + x] = if (x + y) % 9 == 0 { 0xFE_FFD27A } else { 0xFF_B0_5A_3C };
            }
        }
        (px, w, h)
    }

    #[test]
    fn every_code_changes_the_sprite_and_keeps_it_inside_its_box() {
        for code in [1u8, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13] {
            let (mut px, w, h) = block();
            let before = px.clone();
            apply(&mut px, w, h, 0, 0, code, 7);
            assert_eq!(px.len(), w * h);
            assert_ne!(px, before, "code {code} does nothing");
            assert!(px.iter().any(|c| c >> 24 >= 0x80), "code {code} erased the building");
        }
    }

    #[test]
    fn dark_codes_have_no_emissive_pixels_and_machines_have() {
        for code in [2u8, 3, 5, 6, 7, 11, 10, 12, 13] {
            let (mut px, w, h) = block();
            apply(&mut px, w, h, 0, 0, code, 3);
            assert!(!px.iter().any(|c| c >> 24 == 0xFE), "code {code} still glows");
        }
        let (mut px, w, h) = block();
        apply(&mut px, w, h, 0, 0, 4, 3);
        assert!(px.iter().any(|c| c >> 24 == 0xFE));
        let (mut px, w, h) = block();
        apply(&mut px, w, h, 0, 0, 1, 3);
        assert!(px.iter().any(|c| c >> 24 == 0xFE), "a burning building glows");
    }

    #[test]
    fn rubble_is_lower_than_the_building() {
        let (mut px, w, h) = block();
        apply(&mut px, w, h, 0, 0, 2, 5);
        let top = (0..h).find(|&y| (0..w).any(|x| px[y * w + x] >> 24 >= 0x80)).unwrap();
        assert!(top > 25, "a heap, not a building: top row {top}");
    }
}
