//! Things that move in a parade and on the side monitors (B62).

use super::*;

/// A parade float on a flatbed, aligned with the avenue: a skirted deck, a
/// themed centrepiece (a giant flower, a ship's sail, a star arch, a cake),
/// and balloons tied to the corners. `along_x` says which way the avenue runs.
/// The riders are people, drawn by the scene over this.
pub fn float(fb: &mut Framebuffer, cam: &Camera, x: f32, y: f32, level: u8, v: u8, along_x: bool) {
    let base = level as i32 * crate::iso::HEIGHT_UNIT;
    let (hl, hw) = (0.62f32, 0.30f32);
    let r = if along_x { (x - hl, y - hw, x + hl, y + hw) } else { (x - hw, y - hl, x + hw, y + hl) };
    let cloth = [
        palette::rgb(0xd8, 0x3a, 0x3a),
        palette::rgb(0x2f, 0x9a, 0x5a),
        palette::rgb(0xd8, 0xa8, 0x2a),
        palette::rgb(0x8a, 0x4a, 0xc8),
    ][v as usize % 4];
    // Wheels under a skirted deck.
    for k in [-0.4f32, 0.4] {
        let (wx, wy) = if along_x { (x + k, y + hw) } else { (x + hw, y + k) };
        let p = cam.project(wx, wy, base);
        fb.fill_rect(p.0 as i32 - 1, p.1 as i32 - 3, 3, 3, pal::CAR_WHEEL);
    }
    iso_box(fb, cam, r, base, 2, 8, cloth);
    // A fringe of white along the skirt.
    let (fx, fy) = if along_x { (x, y + hw) } else { (x + hw, y) };
    let fp = cam.project(fx, fy, base + 3);
    for dx in -9..=9 {
        fb.put(fp.0 as i32 + dx, fp.1 as i32 + (dx.abs() / 4) as i32 - 1, palette::rgb(0xf4, 0xf0, 0xe0));
    }
    // The centrepiece, standing on the deck's middle.
    let (cx, cy) = cam.project(x, y, base + 8);
    let (cx, cy) = (cx as i32, cy as i32);
    match v % 4 {
        0 => {
            // A giant flower: stem and a ring of petals round a yellow heart.
            fb.fill_rect(cx, cy - 12, 1, 12, palette::rgb(0x3a, 0x84, 0x36));
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let (px, py) = (cx + (a.cos() * 6.0) as i32, cy - 16 + (a.sin() * 5.0) as i32);
                fb.fill_rect(px - 1, py - 1, 3, 3, palette::rgb(0xf0, 0x60, 0x90));
            }
            fb.fill_rect(cx - 2, cy - 18, 5, 5, palette::rgb(0xf8, 0xd0, 0x30));
        }
        1 => {
            // A ship's sail on a mast.
            fb.fill_rect(cx, cy - 22, 1, 22, palette::rgb(0x5a, 0x42, 0x2c));
            for dy in 0..16i32 {
                let w = 1 + dy * 7 / 16;
                for dx in 0..w {
                    fb.put(cx + 1 + dx, cy - 21 + dy, if dy % 5 == 0 { palette::rgb(0xd8, 0xd4, 0xc4) } else { palette::rgb(0xf4, 0xf0, 0xe0) });
                }
            }
            fb.put(cx, cy - 23, palette::rgb(0xd8, 0x3a, 0x3a));
            fb.put(cx + 1, cy - 23, palette::rgb(0xd8, 0x3a, 0x3a));
        }
        2 => {
            // A star on an arch.
            fb.fill_rect(cx - 7, cy - 16, 1, 16, palette::rgb(0xe8, 0xe8, 0xf0));
            fb.fill_rect(cx + 7, cy - 16, 1, 16, palette::rgb(0xe8, 0xe8, 0xf0));
            for dx in -7..=7i32 {
                fb.put(cx + dx, cy - 18 + (dx * dx) / 24, palette::rgb(0xf0, 0xd0, 0x30));
            }
            fb.fill_rect(cx - 1, cy - 25, 3, 5, palette::rgb(0xff, 0xe0, 0x50));
            fb.fill_rect(cx - 3, cy - 23, 7, 2, palette::rgb(0xff, 0xe0, 0x50));
        }
        _ => {
            // A tiered cake with a flame.
            for (i, (w, h)) in [(13i32, 6i32), (9, 5), (5, 4)].iter().enumerate() {
                let y0 = cy - [6, 11, 15][i];
                fb.fill_rect(cx - w / 2, y0 - h + 6, *w, *h, [palette::rgb(0xf4, 0xd8, 0xe0), palette::rgb(0xf0, 0xc0, 0xd0), palette::rgb(0xf8, 0xe8, 0xf0)][i]);
                fb.fill_rect(cx - w / 2, y0 - h + 6, *w, 1, palette::rgb(0xd8, 0x50, 0x60));
            }
            fb.put(cx, cy - 20, palette::rgb(0xff, 0xc8, 0x40));
        }
    }
    // Balloons on strings at the two ends.
    for (k, c) in [(-1.0f32, palette::rgb(0xd8, 0x3a, 0x3a)), (1.0, palette::rgb(0x2f, 0x6a, 0xc8))] {
        let (bx, by) = if along_x { (x + k * hl * 0.9, y) } else { (x, y + k * hl * 0.9) };
        let p = cam.project(bx, by, base + 8);
        let top = (p.0 as i32, p.1 as i32 - 24);
        fb.line(p, (top.0 as f32, top.1 as f32), palette::rgb(0xe8, 0xe4, 0xd8));
        fb.fill_rect(top.0 - 2, top.1 - 4, 5, 5, c);
        fb.put(top.0 - 1, top.1 - 3, palette::shade(c, 1.4));
    }
}

/// A street musician's kit beside the figure the scene draws: a guitar case
/// open on the ground with a few coins, a stool, and notes drifting up. The
/// player stands at `(x, y)`.
pub fn draw_busker(fb: &mut Framebuffer, x: i32, y: i32, t: f32, tint: [f32; 3]) {
    // The open case.
    let case = tinted(palette::rgb(0x2c, 0x2a, 0x30), tint);
    fb.fill_rect(x + 5, y - 1, 8, 3, case);
    fb.fill_rect(x + 6, y, 6, 1, tinted(palette::rgb(0x8a, 0x20, 0x30), tint));
    for dx in [7, 9, 11] {
        fb.put(x + dx, y, palette::rgb(0xe8, 0xc8, 0x50));
    }
    // The guitar across the player's body.
    let wood = tinted(palette::rgb(0xb8, 0x7a, 0x3a), tint);
    fb.line((x as f32 - 3.0, y as f32 - 4.0), (x as f32 + 3.0, y as f32 - 7.0), wood);
    fb.put(x - 3, y - 3, wood);
    fb.put(x - 2, y - 3, wood);
    fb.put(x + 4, y - 8, tinted(palette::rgb(0x5a, 0x3a, 0x1c), tint));
    // Notes.
    draw_notes(fb, x + 2, y - 12, t);
}
