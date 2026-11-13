//! Compare sheet + building crops.
//!
//! The same tile-space region is cut from the 32-px@2× and 64-px@1× renders,
//! so both appear at identical on-screen size, next to a ×2-upscaled crop of
//! `reference/images/6-dense-downtown-plaza.png` (its tiles read ≈32 px, so
//! 2× puts it at the same 64-px on-screen tile pitch, per D55).

use std::error::Error;
use std::path::Path;

use render::framebuffer::Rect;
use render::{Framebuffer, palette};

use crate::col::Mode;
use crate::font;
use crate::pngio;
use crate::scene::{Crop, Scene};

/// The tile-space region the compare sheet zooms on (tower + shop corner + office +
/// townhouse, boulevard + intersection + house).
pub const CMP_REGION: (f32, f32, f32, f32) = (54.0, 56.5, 68.0, 80.5);
pub const SEED: u64 = 0xC17C_2025;

pub struct Renders {
    pub d32: Framebuffer,
    pub n32: Framebuffer,
    pub d64: Framebuffer,
    pub n64: Framebuffer,
    pub crops32: Vec<Crop>,
    pub crops64: Vec<Crop>,
}

fn grid32() -> crate::geom::Grid {
    Scene::new(32, 960, 540, Mode::Day, SEED).g
}

fn grid64() -> crate::geom::Grid {
    Scene::new(64, 1920, 1080, Mode::Day, SEED).g
}

pub fn render_all() -> Renders {
    let mut s32 = Scene::new(32, 960, 540, Mode::Day, SEED);
    let mut d32 = Framebuffer::new(960, 540);
    let crops32 = s32.render(&mut d32);
    s32.mode = Mode::Night;
    let mut n32 = Framebuffer::new(960, 540);
    s32.render(&mut n32);

    let mut s64 = Scene::new(64, 1920, 1080, Mode::Day, SEED);
    let mut d64 = Framebuffer::new(1920, 1080);
    let crops64 = s64.render(&mut d64);
    s64.mode = Mode::Night;
    let mut n64 = Framebuffer::new(1920, 1080);
    s64.render(&mut n64);

    Renders { d32, n32, d64, n64, crops32, crops64 }
}

/// Final on-screen PNGs (1920×1080 both paths).
pub fn write_finals(r: &Renders, dir: &Path) -> Result<(), Box<dyn Error>> {
    for (fb, name) in [(&r.d32, "H25-32px-day.png"), (&r.n32, "H25-32px-night.png")] {
        let mut up = Vec::new();
        render::scale::upscale_into(fb.px(), fb.w, fb.h, &mut up, 2);
        let mut big = Framebuffer::new(fb.w * 2, fb.h * 2);
        big.px_mut().copy_from_slice(&up);
        pngio::write_png(&dir.join(name), &big)?;
    }
    pngio::write_png(&dir.join("H25-64px-day.png"), &r.d64)?;
    pngio::write_png(&dir.join("H25-64px-night.png"), &r.n64)?;
    Ok(())
}

/// Screen bbox of the compare region for a grid, with headroom for the tower.
fn region_rect(g: &crate::geom::Grid, headroom: i32) -> Rect {
    let (x0, y0, x1, y1) = CMP_REGION;
    let mut r = Rect::from_points(&[
        g.p(x0, y0, 0),
        g.p(x1, y0, 0),
        g.p(x1, y1, 0),
        g.p(x0, y1, 0),
    ]);
    r.y0 -= headroom;
    r
}

fn copy_rect(dst: &mut Framebuffer, src: &Framebuffer, r: Rect, dx: i32, dy: i32) {
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            if x >= 0 && y >= 0 && (x as usize) < src.w && (y as usize) < src.h {
                let p = src.px()[y as usize * src.w + x as usize];
                dst.put(dx + (x - r.x0), dy + (y - r.y0), p);
            }
        }
    }
}

fn upscale_rect(dst: &mut Framebuffer, src: &Framebuffer, r: Rect, dx: i32, dy: i32, scale: i32) {
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            if x >= 0 && y >= 0 && (x as usize) < src.w && (y as usize) < src.h {
                let p = src.px()[y as usize * src.w + x as usize];
                for yy in 0..scale {
                    for xx in 0..scale {
                        dst.put(dx + (x - r.x0) * scale + xx, dy + (y - r.y0) * scale + yy, p);
                    }
                }
            }
        }
    }
}

/// 2×-zoom crops of each building at both resolutions (day versions).
pub fn write_crops(r: &Renders, dir: &Path) -> Result<(), Box<dyn Error>> {
    for (list, fb, tag) in [(&r.crops32, &r.d32, "32"), (&r.crops64, &r.d64, "64")] {
        for cr in list {
            let rc = cr.rect.intersect(&fb.bounds());
            if rc.is_empty() {
                continue;
            }
            let w = (rc.width() * 2) as usize;
            let h = (rc.height() * 2) as usize;
            let mut out = Framebuffer::new(w, h);
            upscale_rect(&mut out, fb, rc, 0, 0, 2);
            pngio::write_png(&dir.join(format!("H25-crop-{}-{}.png", cr.name, tag)), &out)?;
        }
    }
    Ok(())
}

/// The compare sheet: day row + night row, three columns each
/// (32-px@2×, 64-px@1×, reference 6 at 2× ≈ same 64-px tile pitch).
pub fn write_compare(r: &Renders, dir: &Path, refimg: &Path) -> Result<(), Box<dyn Error>> {
    let g32 = grid32();
    let r32 = region_rect(&g32, 200).intersect(&r.d32.bounds());
    let cell_w = r32.width() * 2;
    let cell_h = r32.height() * 2;

    let label_h = 14;
    let pad = 8;
    let cols = 3;
    let rows = 2;
    let w = cols * cell_w + (cols + 1) * pad;
    let h = rows * (cell_h + label_h) + (rows + 1) * pad + label_h;
    let mut sheet = Framebuffer::new(w as usize, h as usize);
    sheet.fill_rect(0, 0, w, h, palette::rgb(0x18, 0x1a, 0x1e));

    let white = palette::rgb(0xe8, 0xe8, 0xe8);
    font::text(
        &mut sheet,
        pad,
        2,
        "B25 ART SPIKE - 32PX TILES @2X VS 64PX TILES @1X VS REF6 - SAME ON-SCREEN SIZE",
        white,
        1,
    );

    for row in 0..rows {
        let night = row == 1;
        let y = pad + label_h + row as i32 * (cell_h + label_h + pad);
        let phase = if night { "NIGHT" } else { "DAY" };
        // col 0: 32-px path, upscaled ×2
        let x0 = pad;
        let src = if night { &r.n32 } else { &r.d32 };
        upscale_rect(&mut sheet, src, r32, x0, y, 2);
        font::text(&mut sheet, x0, y - label_h + 3, &format!("32PX @2X {phase}"), white, 1);
        // col 1: 64-px path at 1×, same tile region (grid is exactly 2×)
        let x1 = pad + cell_w + pad;
        let src = if night { &r.n64 } else { &r.d64 };
        let rr = Rect::new(r32.x0 * 2, r32.y0 * 2, r32.x0 * 2 + cell_w, r32.y0 * 2 + cell_h)
            .intersect(&src.bounds());
        copy_rect(&mut sheet, src, rr, x1, y);
        font::text(&mut sheet, x1, y - label_h + 3, &format!("64PX @1X {phase}"), white, 1);
        // col 2: reference crop at 2× (its ~32-px tiles ⇒ same 64-px pitch)
        let x2 = pad + 2 * (cell_w + pad);
        let (rw, rh, rpx) = pngio::read_png(refimg)?;
        let (cx0, cy0) = (255u32, 30u32);
        let cw = (cell_w / 2) as u32;
        let ch = (cell_h / 2) as u32;
        for yy in 0..ch as i32 {
            for xx in 0..cw as i32 {
                let sx = cx0 + xx as u32;
                let sy = cy0 + yy as u32;
                if sx < rw && sy < rh {
                    let p = rpx[(sy * rw + sx) as usize];
                    for dy in 0..2 {
                        for dx in 0..2 {
                            sheet.put(x2 + xx * 2 + dx, y + yy * 2 + dy, p);
                        }
                    }
                }
            }
        }
        if row == 0 {
            font::text(&mut sheet, x2, y - label_h + 3, "REF 6 @2X", white, 1);
        }
    }

    pngio::write_png(&dir.join("H25-compare.png"), &sheet)?;
    Ok(())
}
