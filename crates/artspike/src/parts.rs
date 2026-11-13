//! Procedural parts: wall textures, windows, roofs, awnings, balconies,
//! porches, props (lamps, signs, mailboxes, fences, cars, people) and trees.
//!
//! Everything is drawn at `Grid { tw, s }` resolution: `s` = 1 draws the
//! 32-px-tile detail level, `s` = 2 gets twice the pixels and uses them
//! (brick courses, mullions, balusters, scalloped tiles, needle clusters).

use render::palette::{self, Color};
use render::Framebuffer;

use crate::col::{self, Mode, c};
use crate::geom::{Grid, Quad, box_faces, box_top, corridor_quad};

#[inline]
fn sh(c: Color, f: f32) -> Color {
    palette::shade(c, f)
}

/// Face shading: the +y (screen-left) face is lit, +x (right) is darker.
pub const FACE_L: f32 = 1.0;
pub const FACE_R: f32 = 0.78;
pub const FACE_TOP: f32 = 1.08;

// ---------------------------------------------------------------------------
// Wall textures (closures passed to Quad::paint)
// ---------------------------------------------------------------------------

/// Clapboard siding: horizontal courses with a shadow under each lap.
pub fn paint_clap(q: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32, s: i32) {
    let row = (2 * s).max(2);
    q.paint(fb, &mut |u, v, x, y| {
        let lap = (v % row) == row - 1; // bottom row of each course = shadow line
        let mut f = if lap { 0.86 } else { 1.0 };
        if s >= 2 && (v % row) == 0 {
            f = 1.05; // sun catching the top of the lap
        }
        // faint vertical butt joints
        if s >= 2 && u % (7 * s) == 0 && v % (row * 2) < row {
            f *= 0.93;
        }
        let jit = col::jcell(x, y, 2 * s, 0.028);
        mode.surf(sh(base, f * side * jit))
    });
}

/// Brick: courses with staggered vertical joints.
pub fn paint_brick(q: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32, s: i32) {
    let course = (2 * s).max(2);
    let bw = (4 * s).max(3);
    q.paint(fb, &mut |u, v, x, y| {
        let mortar_v = (v % course) == course - 1;
        let stagger = if (v / course) % 2 == 0 { 0 } else { bw / 2 };
        let mortar_u = s >= 2 && ((u + stagger) % bw) == bw - 1;
        // per-brick tint so the wall reads as laid brick, not noise
        let jit = 0.94 + 0.10 * col::hash01(u / bw, v / course);
        let shade2 = col::jcell(x, y, 6 * s, 0.03);
        let c = if mortar_v || mortar_u {
            c::BRICK_MORTAR
        } else {
            sh(base, jit * shade2)
        };
        mode.surf(sh(c, side))
    });
}

/// Stucco: flat with sparse weathering blotches.
pub fn paint_stucco(q: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32, s: i32) {
    q.paint(fb, &mut |_u, v, x, y| {
        let mut f = col::jcell(x, y, (3 * s).max(2), 0.035);
        if col::blotch(x, y, 3 * s, 29) {
            f *= 0.94;
        }
        if v == 0 {
            f = 1.04;
        }
        mode.surf(sh(base, f * side))
    });
}

/// Flat painted surface with faint noise (concrete, painted trim).
pub fn paint_flat(q: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32) {
    q.paint(fb, &mut |_u, _v, x, y| {
        let jit = col::jcell(x, y, 3, 0.025);
        mode.surf(sh(base, jit * side))
    });
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// One window on a face: frame, glass, sill, optional shutters / lit state.
/// `lit` at night draws the warm/cool emissive; during the day glass is
/// dark with a sky glint.
#[derive(Clone, Copy)]
pub struct WinOpts {
    pub w: i32,
    pub h: i32,
    pub frame: Color,
    pub sill: bool,
    pub shutters: bool,
    pub shutter_c: Color,
    /// hash salt for the per-window lit decision
    pub seed: u64,
    pub lit_p: f32,
}

#[allow(clippy::too_many_arguments)]
pub fn window(
    q: &Quad,
    fb: &mut Framebuffer,
    mode: Mode,
    upx: i32,
    vpx: i32,
    o: WinOpts,
    s: i32,
) {
    let rows = q.rows() as f32;
    let wq = q.sub_px(upx, vpx, upx + o.w, vpx + o.h, rows);
    let frame = mode.surf(sh(o.frame, 1.0));
    let lit = {
        let h = col::hash2(upx + o.seed as i32, vpx * 31 + o.seed as i32);
        (h % 1000) as f32 / 1000.0
    };
    let glass = match mode {
        Mode::Day => {
            if lit < 0.25 {
                sh(c::GLASS_HI, 0.9) // sky reflection
            } else {
                col::WIN_OFF
            }
        }
        Mode::Night => {
            if lit < o.lit_p {
                if lit < o.lit_p * 0.18 {
                    col::WIN_COOL
                } else {
                    col::WIN_WARM
                }
            } else {
                sh(col::WIN_OFF, 0.8)
            }
        }
    };
    let night_lit = mode == Mode::Night && (glass == col::WIN_WARM || glass == col::WIN_COOL);
    wq.paint(fb, &mut |u, v, _x, _y| {
        let border = u == 0 || v == 0 || u == wq_w(o.w) - 1 || v == o.h - 1;
        if border {
            return frame;
        }
        // glazing bar down the middle on fine resolutions
        if s >= 2 && o.w >= 5 && u == o.w / 2 {
            return frame;
        }
        if night_lit {
            glass
        } else {
            sh(glass, 0.95 + 0.08 * col::hash01(u, v))
        }
    });
    if o.sill {
        let sq = q.sub_px(upx - 1, vpx + o.h, upx + o.w + 1, vpx + o.h + s, rows);
        sq.fill(fb, frame);
    }
    if o.shutters && o.w >= 3 {
        let sw = (s).max(1);
        let lq = q.sub_px(upx - sw - 1, vpx, upx - 1, vpx + o.h, rows);
        let rq = q.sub_px(upx + o.w + 1, vpx, upx + o.w + sw + 1, vpx + o.h, rows);
        lq.fill(fb, mode.surf(o.shutter_c));
        rq.fill(fb, mode.surf(o.shutter_c));
    }
    // warm halo for lit windows at night
    if night_lit {
        halo_rect(fb, &wq, mode, sh(glass, 0.9), 0.25);
    }
}

#[inline]
fn wq_w(w: i32) -> i32 {
    w
}

/// A grid of identical windows on a wall face.
#[allow(clippy::too_many_arguments)]
pub fn window_grid(
    face: &Quad,
    fb: &mut Framebuffer,
    mode: Mode,
    cols: i32,
    floors: i32,
    margin_u: i32,
    top_v: i32,
    floor_h: i32,
    o: WinOpts,
    s: i32,
) {
    let face_w = face.top_len();
    let usable = face_w - 2 * margin_u;
    let step = usable / cols.max(1);
    for f in 0..floors {
        for cx in 0..cols {
            let u = margin_u + cx * step + (step - o.w) / 2;
            let v = top_v + f * floor_h + (floor_h - o.h) / 2 - (if o.sill { 0 } else { 0 });
            let mut oo = o;
            oo.seed = o.seed + (f * 131 + cx * 17) as u64;
            window(face, fb, mode, u, v, oo, s);
        }
    }
}

// ---------------------------------------------------------------------------
// Roofs
// ---------------------------------------------------------------------------

/// Flat roof: deck + parapet lip (front two edges), optional plant.
#[allow(clippy::too_many_arguments)]
pub fn roof_flat(
    fb: &mut Framebuffer,
    g: &Grid,
    mode: Mode,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    deck: Color,
    parapet: Color,
    units: bool,
    seed: u64,
) {
    let top = box_top(g, x0, y0, x1, y1, z);
    top.paint(fb, &mut |_u, v, x, y| {
        let jit = col::jcell(x, y, (3 * g.s).max(2), 0.05);
        // faint roofing felt strips
        let strip = if (v / (3 * g.s).max(2)) % 2 == 0 { 0.96 } else { 1.0 };
        mode.surf(sh(deck, jit * strip * FACE_TOP))
    });
    // parapet: the two near faces, 1-2 px
    let ph = (2 * g.s).max(2);
    let (l, r) = box_faces(g, x0, y0, x1, y1, z - ph, z);
    // only the thin parapet faces; they occlude the roof edge
    l.paint(fb, &mut |_u, v, _x, _y| mode.surf(sh(parapet, if v == 0 { 1.1 } else { 0.95 })));
    r.paint(fb, &mut |_u, v, _x, _y| {
        mode.surf(sh(parapet, if v == 0 { 1.05 } else { 0.78 }))
    });
    if units && g.s >= 1 {
        // rooftop plant: AC condensers and a vent stack
        let mut rng = map::rng::Rng::new(seed);
        let n = ((x1 - x0) * (y1 - y0) * 0.35) as i32 + 1;
        for _ in 0..n.min(6) {
            let ux = x0 + 0.35 + rng.f32() * ((x1 - x0) - 0.8).max(0.2);
            let uy = y0 + 0.35 + rng.f32() * ((y1 - y0) - 0.8).max(0.2);
            let uh = (2 * g.s) + rng.range(0, 2);
            let (lf, rf) = box_faces(g, ux, uy, ux + 0.3, uy + 0.3, z, z + uh);
            let tq = box_top(g, ux, uy, ux + 0.3, uy + 0.3, z + uh);
            lf.fill(fb, mode.surf(sh(c::ROOF_UNIT, 0.95)));
            rf.fill(fb, mode.surf(sh(c::ROOF_UNIT, FACE_R)));
            tq.fill(fb, mode.surf(sh(c::ROOF_UNIT, 1.15)));
            // fan dot at fine scale
            if g.s >= 2 {
                let (px, py) = g.p(ux + 0.15, uy + 0.15, z + uh + 1);
                fb.put(px as i32, py as i32, mode.surf(c::MULLION));
            }
        }
    }
}

/// Gable roof: two shingled planes meeting at a ridge along the +x axis
/// (the two visible slopes face +y-left and +x-right).
#[allow(clippy::too_many_arguments)]
pub fn roof_gable(
    fb: &mut Framebuffer,
    g: &Grid,
    mode: Mode,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
    base: Color,
) {
    let midy = (y0 + y1) / 2.0;
    // slope toward +y (screen-left face)
    let sl = Quad::new(
        g.p(x0, midy, z + rise),
        g.p(x1, midy, z + rise),
        g.p(x1, y1, z),
        g.p(x0, y1, z),
    );
    // slope toward +x (screen-right face) — only visible if the ridge is
    // behind it; in our iso both slopes of an x-ridge gable show.
    let rl = Quad::new(
        g.p(x0, midy, z + rise),
        g.p(x1, midy, z + rise),
        g.p(x1, y0, z),
        g.p(x0, y0, z),
    );
    paint_shingles(&sl, fb, mode, base, 1.0, g.s);
    paint_shingles(&rl, fb, mode, sh(base, 0.9), 0.86, g.s);
    // ridge cap
    let (ax, ay) = g.p(x0, midy, z + rise);
    let (bx, by) = g.p(x1, midy, z + rise);
    fb.line((ax, ay), (bx, by), mode.surf(sh(base, 0.55)));
}

fn paint_shingles(q: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32, s: i32) {
    let course = (2 * s).max(2);
    let sw = (3 * s).max(2);
    q.paint(fb, &mut |u, v, _x, _y| {
        let gap = (v % course) == course - 1;
        let stagger = if (v / course) % 2 == 0 { 0 } else { sw / 2 };
        let joint = s >= 2 && ((u + stagger) % sw) == 0;
        let f = if gap {
            0.72
        } else if joint {
            0.85
        } else {
            0.95 + 0.08 * col::hash01(u / sw, v * 13 / course)
        };
        mode.surf(sh(base, f * side * FACE_TOP * 0.96))
    });
}

/// Hip roof with scalloped clay tiles (the townhouse).
#[allow(clippy::too_many_arguments)]
pub fn roof_hip_tile(
    fb: &mut Framebuffer,
    g: &Grid,
    mode: Mode,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z: i32,
    rise: i32,
) {
    let mx = (x0 + x1) / 2.0;
    let my = (y0 + y1) / 2.0;
    if (x1 - x0) > (y1 - y0) + 0.01 {
        // ridge along x
        let rl = Quad::new(g.p(x0 + 0.6, my, z + rise), g.p(x1 - 0.6, my, z + rise), g.p(x1, y1, z), g.p(x0, y1, z));
        let rr = Quad::new(g.p(x0 + 0.6, my, z + rise), g.p(x1 - 0.6, my, z + rise), g.p(x1, y0, z), g.p(x0, y0, z));
        let hipl = Quad::new(g.p(x0 + 0.6, my, z + rise), g.p(x0, y0, z), g.p(x0, y1, z), g.p(x0, y1, z));
        let hipr = Quad::new(g.p(x1 - 0.6, my, z + rise), g.p(x1, y0, z), g.p(x1, y1, z), g.p(x1, y1, z));
        paint_tiles(&rl, fb, mode, 1.0, g.s);
        paint_tiles(&rr, fb, mode, 0.84, g.s);
        paint_tiles(&hipl, fb, mode, 0.92, g.s);
        paint_tiles(&hipr, fb, mode, 0.8, g.s);
        let (ax, ay) = g.p(x0 + 0.6, my, z + rise);
        let (bx, by) = g.p(x1 - 0.6, my, z + rise);
        fb.line((ax, ay), (bx, by), mode.surf(c::TILE_ROOF_DARK));
    } else {
        // pyramid-ish (square footprint)
        let fl = Quad::new(g.p(mx, my, z + rise), g.p(mx, my, z + rise), g.p(x1, y1, z), g.p(x0, y1, z));
        let fr = Quad::new(g.p(mx, my, z + rise), g.p(mx, my, z + rise), g.p(x1, y0, z), g.p(x1, y1, z));
        paint_tiles(&fl, fb, mode, 1.0, g.s);
        paint_tiles(&fr, fb, mode, 0.84, g.s);
        let (px, py) = g.pi_f(mx, my, z + rise);
        fb.put(px, py, mode.surf(c::TILE_ROOF_DARK));
    }
}

fn paint_tiles(q: &Quad, fb: &mut Framebuffer, mode: Mode, side: f32, s: i32) {
    let course = (2 * s).max(2);
    let tw = (3 * s).max(2);
    q.paint(fb, &mut |u, v, _x, _y| {
        let row_shadow = (v % course) == course - 1;
        let stagger = if (v / course) % 2 == 0 { 0 } else { tw / 2 };
        let seam = s >= 2 && ((u + stagger) % tw) == 0;
        let f = if row_shadow {
            0.7
        } else if seam {
            0.8
        } else {
            0.94 + 0.1 * col::hash01(u / tw, v * 11 / course)
        };
        mode.surf(sh(c::TILE_ROOF, f * side * FACE_TOP * 0.95))
    });
}

// ---------------------------------------------------------------------------
// Ground: grass, roads, sidewalks, markings
// ---------------------------------------------------------------------------

/// One grass diamond with sub-tile variation and mottled patches.
pub fn grass_tile(fb: &mut Framebuffer, g: &Grid, tx: i32, ty: i32, mode: Mode) {
    let q = crate::geom::tile_quad(g, tx, ty);
    let checker = (tx + ty) % 2 == 0;
    let base = if checker { c::GRASS_A } else { c::GRASS_B };
    q.paint(fb, &mut |u, v, x, y| {
        // macro patches (per ~5-px cell) + fine blade speckle (per 2-px cell)
        let patch = col::jcell(x, y, 5, 0.07);
        let fine = if col::blotch(x, y, 2, 31) { 0.94 } else { 1.0 };
        let _ = (u, v, tx, ty);
        mode.surf(sh(base, patch * fine))
    });
}

/// Raw asphalt corridor; markings are layered over it by the caller.
pub fn asphalt(fb: &mut Framebuffer, g: &Grid, q: &Quad, mode: Mode) {
    q.paint(fb, &mut |u, v, x, y| {
        // large soft patches + at fine scale a faint aggregate speckle
        let f = col::jcell(x, y, 8 * g.s, 0.05) * col::jcell(x, y, 2, 0.018);
        let sp = if g.s >= 2 && col::blotch(x, y, 3, 53) { 1.07 } else { 1.0 };
        let _ = (u, v);
        mode.surf(sh(c::ASPHALT, f * sp))
    });
}

/// Sidewalk: pavers with joints and a curb line on the road side.
pub fn sidewalk(fb: &mut Framebuffer, g: &Grid, q: &Quad, mode: Mode, curb_v: i32) {
    let joint = (4 * g.s).max(3);
    q.paint(fb, &mut |u, v, x, y| {
        if v == curb_v {
            return mode.surf(sh(c::CURB, 1.06));
        }
        let j = (u % joint) == 0 || (v % joint) == 0;
        let jit = col::jcell(x, y, joint, 0.045);
        if j {
            mode.surf(c::SIDEWALK_LINE)
        } else {
            mode.surf(sh(c::SIDEWALK, jit))
        }
    });
}

/// A manhole cover.
pub fn manhole(fb: &mut Framebuffer, g: &Grid, tx: f32, ty: f32, mode: Mode) {
    let (cx, cy) = g.pi_f(tx, ty, 1);
    let r = g.s;
    for dy in -r..=r {
        for dx in -(2 * r)..=(2 * r) {
            let d = (dx * dx) as f32 / 4.0 + (dy * dy) as f32;
            if d <= (r * r) as f32 + 0.5 {
                fb.put(cx + dx, cy + dy, mode.surf(c::MANHOLE));
            }
        }
    }
    if g.s >= 2 {
        fb.put(cx, cy, mode.surf(sh(c::MANHOLE, 1.35)));
    }
}

/// Solid white stop bar before a crosswalk.
pub fn stop_bar(fb: &mut Framebuffer, g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, mode: Mode) {
    let q = corridor_quad(g, x0, y0, x1, y1, 1);
    q.fill(fb, mode.surf(c::CROSSWALK));
}

/// Zebra crosswalk across a corridor running along x (stripes across it).
pub fn crosswalk_x(fb: &mut Framebuffer, g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, mode: Mode) {
    let q = corridor_quad(g, x0, y0, x1, y1, 1);
    let stripe = (3 * g.s).max(2);
    q.paint(fb, &mut |u, _v, _x, _y| {
        if (u % (stripe * 2)) < stripe {
            mode.surf(c::CROSSWALK)
        } else {
            mode.surf(c::ASPHALT)
        }
    });
}

/// Zebra across a corridor running along y.
pub fn crosswalk_y(fb: &mut Framebuffer, g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, mode: Mode) {
    let q = corridor_quad(g, x0, y0, x1, y1, 1);
    let stripe = (3 * g.s).max(2);
    q.paint(fb, &mut |_u, v, _x, _y| {
        if (v % (stripe * 2)) < stripe {
            mode.surf(c::CROSSWALK)
        } else {
            mode.surf(c::ASPHALT)
        }
    });
}

/// Dashed lane line along a corridor running along x, at v-row `vrow`
/// (corridor-local px). Dashes along u.
pub fn lane_dashes_x(fb: &mut Framebuffer, g: &Grid, q: &Quad, vrow: i32, colr: Color, mode: Mode) {
    let dash = (5 * g.s).max(3);
    let gap = (4 * g.s).max(3);
    let rows = q.rows();
    let vv = (vrow).clamp(0, rows - 1);
    let (lx, ly) = q.pt(0.0, vv as f32, rows as f32);
    let (rx, ry) = q.pt(1.0, vv as f32, rows as f32);
    let steps = ((rx - lx).abs().max((ry - ly).abs()).round() as i32).max(1);
    for i in 0..=steps {
        if (i % (dash + gap)) < dash {
            let x = (lx + (rx - lx) * i as f32 / steps as f32).round() as i32;
            let y = (ly + (ry - ly) * i as f32 / steps as f32).round() as i32;
            fb.put(x, y - 1, mode.surf(colr));
        }
    }
}

/// Same for a corridor along y (dashes along v).
pub fn lane_dashes_y(fb: &mut Framebuffer, g: &Grid, q: &Quad, urow: i32, colr: Color, mode: Mode) {
    let _ = g;
    let dash = 6;
    let gap = 4;
    let rows = q.rows();
    for vv in 0..rows {
        if (vv % (dash + gap)) < dash {
            let (x, y) = q.pt(urow as f32 / q.top_len() as f32, vv as f32, rows as f32);
            fb.put(x.round() as i32, y.round() as i32 - 1, mode.surf(colr));
        }
    }
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

/// Radial glow, alpha-blended. Used for lamp pools, neon halos, headlights.
pub fn glow(fb: &mut Framebuffer, x: i32, y: i32, rx: i32, ry: i32, peak: f32, colr: Color) {
    let rx = rx.max(1);
    let ry = ry.max(1);
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            let d = ((dx * dx) as f32 / (rx * rx) as f32 + (dy * dy) as f32 / (ry * ry) as f32).sqrt();
            if d < 1.0 {
                let cov = peak * (1.0 - d) * (1.0 - d);
                fb.put_blended(x + dx, y + dy, colr, cov);
            }
        }
    }
}

/// Halo ring around an emissive quad (neon/billboard/lit window).
fn halo_rect(fb: &mut Framebuffer, q: &Quad, _mode: Mode, colr: Color, peak: f32) {
    let r = crate::geom::quad_bounds(q).expanded(2);
    let b = crate::geom::quad_bounds(q);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let inside = x >= b.x0 && x < b.x1 && y >= b.y0 && y < b.y1;
            if !inside {
                fb.put_blended(x, y, colr, peak * 0.5);
            }
        }
    }
}

/// Street lamp: post + head; at night a lit head and a pool on the ground.
pub fn lamp_post(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let h = 7 * g.s;
    let post = mode.surf(c::LAMP_POST);
    for dy in 0..h {
        fb.put(bx, by - dy, post);
        if g.s >= 2 {
            fb.put(bx + 1, by - dy, sh(post, 0.8));
        }
    }
    match mode {
        Mode::Day => {
            fb.put(bx, by - h, mode.surf(sh(c::LAMP_POST, 1.3)));
            fb.put(bx + 1, by - h, mode.surf(sh(c::LAMP_POST, 1.3)));
        }
        Mode::Night => {
            let head = col::LAMP;
            glow(fb, bx, by - h, 3 * g.s, 2 * g.s, 0.5, head);
            fb.put(bx, by - h, head);
            fb.put(bx + 1, by - h, head);
            // the pool of light on the ground
            glow(fb, bx + 1, by, 8 * g.s, 4 * g.s, 0.30, head);
        }
    }
}

/// Street name sign on a post.
pub fn street_sign(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let h = 5 * g.s;
    for dy in 0..h {
        fb.put(bx, by - dy, mode.surf(c::LAMP_POST));
    }
    let plate = mode.surf(rgb(0x2c, 0x6e, 0x49));
    let w = 5 * g.s;
    for dy in 0..2 * g.s {
        for dx in 0..w {
            fb.put(bx + 1 + dx, by - h + dy, plate);
        }
    }
    // white lettering suggestion
    for dx in 1..w - 1 {
        if dx % 2 == 0 {
            fb.put(bx + 1 + dx, by - h + (2 * g.s) / 2, mode.surf(palette::rgb(0xe8, 0xee, 0xe8)));
        }
    }
}

use render::palette::rgb;

/// Traditional red fire hydrant with silver caps.
pub fn fire_hydrant(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    let red = mode.surf(c::HYDRANT);
    let silver = mode.surf(c::TRIM);
    for dy in 0..3 * s {
        fb.put(bx, by - dy, red);
        if s >= 2 {
            fb.put(bx + 1, by - dy, sh(red, 0.85));
            fb.put(bx - 1, by - dy, sh(red, 1.1));
        }
    }
    // top nut
    fb.put(bx, by - 3 * s, silver);
    // side nozzle
    if s >= 2 {
        fb.put(bx + 2, by - 2 * s, silver);
        fb.put(bx - 2, by - 2 * s, silver);
    }
}

/// Street bench with wooden slats and cast-iron frame.
pub fn park_bench(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, dir: char) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    let wood = mode.surf(c::BENCH);
    let iron = mode.surf(c::LAMP_POST);
    let len = 4 * s;
    if dir == 'x' {
        for i in 0..len {
            let px = bx + i;
            let py = by + i / 2;
            fb.put(px, py - 2 * s, wood);
            fb.put(px, py - 3 * s, sh(wood, 1.15));
            if i == 0 || i == len - 1 {
                fb.put(px, py - s, iron);
                fb.put(px, py, iron);
            }
        }
    } else {
        for i in 0..len {
            let px = bx - i;
            let py = by + i / 2;
            fb.put(px, py - 2 * s, wood);
            fb.put(px, py - 3 * s, sh(wood, 1.15));
            if i == 0 || i == len - 1 {
                fb.put(px, py - s, iron);
                fb.put(px, py, iron);
            }
        }
    }
}

/// Paved walkway from porch to sidewalk (flagstone / brick pavers).
pub fn walkway(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, y0: f32, x1: f32, y1: f32) {
    let q = corridor_quad(g, x0, y0, x1, y1, 1);
    let s = g.s;
    let step = (3 * s).max(2);
    q.paint(fb, &mut |u, v, x, y| {
        let seam = (u % step) == 0 || (v % step) == 0;
        let jit = 0.94 + 0.12 * col::hash01(x, y);
        if seam {
            mode.surf(sh(c::SIDEWALK_LINE, 0.9))
        } else if (u / step + v / step) % 2 == 0 {
            mode.surf(sh(c::PAVER_A, jit))
        } else {
            mode.surf(sh(c::PAVER_B, jit))
        }
    });
}

/// US-style mailbox on a post.
pub fn mailbox(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    fb.put(bx, by, mode.surf(sh(c::MAILBOX, 0.7)));
    fb.put(bx, by - 1, mode.surf(sh(c::MAILBOX, 0.7)));
    for dy in 0..2 * g.s {
        for dx in 0..2 * g.s {
            fb.put(bx + dx, by - 2 - dy, mode.surf(c::MAILBOX));
        }
    }
    // red flag
    fb.put(bx + 2 * g.s, by - 2 - 2 * g.s, mode.surf(rgb(0xd6, 0x3c, 0x30)));
}

/// A picket fence run along the +x edge at tile y, from x0 to x1.
pub fn fence_x(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, x1: f32, ty: f32, colr: Color) {
    let (ax, ay) = g.pi_f(x0, ty, 0);
    let (bx, by) = g.pi_f(x1, ty, 0);
    let steps = ((bx - ax).abs().max((by - ay).abs())).max(1);
    let h = 2 * g.s + 1;
    for i in 0..=steps {
        let x = ax + (bx - ax) * i / steps;
        let y = ay + (by - ay) * i / steps;
        if i % (2 * g.s) == 0 {
            for dy in 0..h {
                fb.put(x, y - dy, mode.surf(colr));
            }
        } else {
            fb.put(x, y - h / 2, mode.surf(sh(colr, 0.92)));
            fb.put(x, y - h + 1, mode.surf(sh(colr, 0.92)));
        }
    }
}

/// Same along the +y edge at tile x.
pub fn fence_y(fb: &mut Framebuffer, g: &Grid, mode: Mode, y0: f32, y1: f32, tx: f32, colr: Color) {
    let (ax, ay) = g.pi_f(tx, y0, 0);
    let (bx, by) = g.pi_f(tx, y1, 0);
    let steps = ((bx - ax).abs().max((by - ay).abs())).max(1);
    let h = 2 * g.s + 1;
    for i in 0..=steps {
        let x = ax + (bx - ax) * i / steps;
        let y = ay + (by - ay) * i / steps;
        if i % (2 * g.s) == 0 {
            for dy in 0..h {
                fb.put(x, y - dy, mode.surf(colr));
            }
        } else {
            fb.put(x, y - h / 2, mode.surf(sh(colr, 0.92)));
            fb.put(x, y - h + 1, mode.surf(sh(colr, 0.92)));
        }
    }
}

/// A garden bed: soil rows with veg/flower dots.
pub fn garden(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, y0: f32, x1: f32, y1: f32) {
    let q = corridor_quad(g, x0, y0, x1, y1, 1);
    let row = (3 * g.s).max(2);
    q.paint(fb, &mut |_u, v, x, y| {
        let in_row = (v % row) == 1;
        let h = col::hash2(x, y);
        if in_row {
            let pick = h % 5;
            let veg = match pick {
                0 => c::FLOWER_A,
                1 => c::FLOWER_B,
                2 => c::FLOWER_C,
                _ => sh(c::GRASS_A, 1.1),
            };
            mode.surf(sh(veg, 0.9 + 0.15 * col::hash01(y, x)))
        } else {
            mode.surf(sh(c::GARDEN_SOIL, 0.9 + 0.12 * col::hash01(x, y)))
        }
    });
}

/// A parked/driving car at a tile point, facing along +x ("x") or +y ("y").
pub fn car(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, dir: char, body: Color, seed: u64) {
    let _ = seed;
    let s = g.s;
    ground_shadow(fb, g, tx, ty, 0.45, mode);
    let (l, hln) = if dir == 'x' { (0.62f32, 0.36f32) } else { (0.36, 0.62) };
    let x0 = tx - l / 2.0;
    let y0 = ty - hln / 2.0;
    let bh = (2 * g.s).max(1) + 2;
    let outline = mode.surf(sh(body, 0.42));
    // body
    let (lf, rf) = box_faces(g, x0, y0, x0 + l, y0 + hln, 0, bh);
    let flen = lf.top_len();
    lf.paint(fb, &mut |u, v, _x, _y| {
        let bot = v >= lf.rows() - 2;
        if bot && (u < 3 || u >= flen - 3) {
            mode.surf(palette::rgb(0x20, 0x20, 0x24)) // wheels
        } else if bot || v == 0 {
            mode.surf(sh(body, 0.72))
        } else {
            mode.surf(sh(body, 1.0 + 0.04 * col::hash01(u, v)))
        }
    });
    rf.paint(fb, &mut |u, v, _x, _y| {
        let bot = v >= rf.rows() - 2;
        if bot && (u < 3 || u >= flen - 3) {
            mode.surf(palette::rgb(0x20, 0x20, 0x24))
        } else if bot || v == 0 {
            mode.surf(sh(body, 0.62))
        } else {
            mode.surf(sh(body, FACE_R))
        }
    });
    let tq = box_top(g, x0, y0, x0 + l, y0 + hln, bh);
    tq.paint(fb, &mut |u, v, _, _| {
        if u < 2 || v < 2 || u >= tq.top_len() - 2 || v >= tq.rows() - 2 {
            outline
        } else {
            mode.surf(sh(body, 1.08))
        }
    });
    // cabin: glass band with body pillars
    let ch = (2 * g.s).max(2);
    let cx0 = x0 + l * 0.24;
    let cx1 = x0 + l * 0.76;
    let cy0 = y0 + hln * 0.16;
    let cy1 = y0 + hln * 0.84;
    let (clf, crf) = box_faces(g, cx0, cy0, cx1, cy1, bh, bh + ch);
    let cw = clf.top_len();
    clf.paint(fb, &mut |u, v, _, _| {
        if u < 2 || u >= cw - 2 || (cw > 8 && (u - cw / 2).abs() < 1) {
            mode.surf(sh(body, 0.95)) // pillars
        } else {
            let _ = v;
            mode.surf(c::CAR_GLASS)
        }
    });
    crf.fill(fb, mode.surf(sh(c::CAR_GLASS, FACE_R)));
    let ct = box_top(g, cx0, cy0, cx1, cy1, bh + ch);
    ct.fill(fb, mode.surf(sh(body, 1.14)));
    if mode == Mode::Night {
        let (hx, hy) = g.pi_f(if dir == 'x' { x0 + l } else { tx }, if dir == 'x' { ty } else { y0 + hln }, 1);
        glow(fb, hx, hy, 3 * s, 2 * s, 0.8, rgb(0xff, 0xf4, 0xd0));
        fb.put(hx, hy, rgb(0xff, 0xff, 0xee));
        let (fx, fy) = if dir == 'x' {
            (hx + 8 * s, hy + 4 * s)
        } else {
            (hx - 8 * s, hy + 4 * s)
        };
        glow(fb, fx, fy, 10 * s, 5 * s, 0.35, rgb(0xff, 0xf0, 0xc0));
        let (tx_pos, ty_pos) = g.pi_f(if dir == 'x' { x0 } else { tx }, if dir == 'x' { ty } else { y0 }, 1);
        fb.put(tx_pos, ty_pos - s, rgb(0xff, 0x24, 0x18));
        glow(fb, tx_pos, ty_pos - s, 2 * s, s, 0.3, rgb(0xff, 0x20, 0x10));
    }
}

/// A person: head, torso, legs. 2×5 px at s=1, 3×9 at s=2.
pub fn person(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, shirt: Color, seed: u64) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    let h = col::hash2(seed as i32, bx + by);
    let skin = if h % 4 == 0 { rgb(0x8a, 0x5c, 0x3c) } else { rgb(0xe8, 0xc4, 0xa0) };
    let leg = mode.surf(rgb(0x36, 0x38, 0x42));
    let shirt = mode.surf(shirt);
    let skin = mode.surf(skin);
    if s == 1 {
        fb.put(bx, by, leg);
        fb.put(bx, by - 1, leg);
        fb.put(bx, by - 2, shirt);
        fb.put(bx + 1, by - 2, shirt);
        fb.put(bx, by - 3, shirt);
        fb.put(bx, by - 4, skin);
        fb.put(bx + 1, by - 4, skin);
    } else {
        // legs
        fb.put(bx, by, leg);
        fb.put(bx - 1, by, leg);
        fb.put(bx + 1, by - 1, leg);
        fb.put(bx - 1, by - 1, leg);
        // torso 3×3
        for dy in 2..5 {
            for dx in -1..=1 {
                fb.put(bx + dx, by - dy, shirt);
            }
        }
        // arms slightly darker
        fb.put(bx - 1, by - 3, sh(shirt, 0.85));
        fb.put(bx + 1, by - 4, sh(shirt, 0.85));
        // head 2×2 + hair
        fb.put(bx, by - 5, skin);
        fb.put(bx - 1, by - 6, skin);
        fb.put(bx + 1, by - 6, skin);
        fb.put(bx, by - 6, skin);
        fb.put(bx, by - 7, mode.surf(rgb(0x4a, 0x36, 0x28)));
        fb.put(bx - 1, by - 7, mode.surf(rgb(0x4a, 0x36, 0x28)));
    }
}

// ---------------------------------------------------------------------------
// Trees & Shadows
// ---------------------------------------------------------------------------

/// Directional isometric cast shadow for buildings.
/// Sun illuminates the top-left (+y face); shadow casts down-right (+x axis).
pub fn building_shadow(fb: &mut Framebuffer, g: &Grid, x0: f32, y0: f32, x1: f32, y1: f32, h: i32, mode: Mode) {
    let dx = (h as f32) / (g.hw() as f32 * 1.5);
    let pts = [
        g.p(x0, y1, 0),
        g.p(x0 + dx, y1, 0),
        g.p(x0 + dx, y0, 0),
        g.p(x1 + dx, y0, 0),
        g.p(x1, y0, 0),
        g.p(x1, y1, 0),
    ];
    let colr = match mode {
        Mode::Day => rgb(0x18, 0x24, 0x1c),
        Mode::Night => rgb(0x06, 0x0a, 0x14),
    };
    let cov = match mode {
        Mode::Day => 0.28,
        Mode::Night => 0.35,
    };
    crate::geom::fill_poly_blended(fb, &pts, colr, cov);
}

/// Soft ground shadow under a tree/prop object, cast toward screen-right.
pub fn ground_shadow(fb: &mut Framebuffer, g: &Grid, tx: f32, ty: f32, r: f32, mode: Mode) {
    let s = g.s;
    // Shift slightly in the sun shadow direction (+x tile direction)
    let (cx, cy) = g.pi_f(tx + 0.3 * r, ty, 0);
    let rx = ((r * g.tw as f32 * 0.45) as i32).max(2 * s);
    let ry = ((r * g.tw as f32 * 0.24) as i32).max(s + 1);
    let dark = match mode {
        Mode::Day => rgb(0x1a, 0x2c, 0x18),
        Mode::Night => rgb(0x06, 0x0c, 0x18),
    };
    glow(fb, cx, cy, rx, ry, if mode == Mode::Day { 0.32 } else { 0.40 }, dark);
}

/// Round-canopy oak: lush 4-tone leaf clusters with branches and shadow.
pub fn tree_oak(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, seed: u64, scale: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    ground_shadow(fb, g, tx, ty, 0.7 * scale, mode);
    let trunk = mode.surf(c::TRUNK_DARK);
    let th = (3.5 * s as f32 * scale) as i32;
    for dy in 0..th {
        fb.put(bx, by - dy, trunk);
        if s >= 2 {
            fb.put(bx + 1, by - dy, trunk);
            fb.put(bx - 1, by - dy, sh(trunk, 0.8));
        }
    }
    // branch forks
    if s >= 2 {
        fb.put(bx - 2, by - th, trunk);
        fb.put(bx + 2, by - th, trunk);
    }
    let cy = by - (5.5 * s as f32 * scale) as i32;
    let r = ((4.5 * scale) as i32 * s).max(4);
    let leaf = [
        rgb(0x3e, 0x8e, 0x34), // mid-dark
        rgb(0x4e, 0xa6, 0x40), // mid-light
        rgb(0x72, 0xc6, 0x54), // highlight
        rgb(0x28, 0x68, 0x22), // deep shadow
    ];
    for dy in -r..=r {
        for dx in -(r * 7 / 5)..=(r * 7 / 5) {
            let d = (dx * dx) as f32 / 1.5 + (dy * dy) as f32;
            if d <= (r * r) as f32 {
                let h = col::hash2(seed as i32 + dx * 3, (seed / 7) as i32 + dy * 5);
                let shade_side = dx + dy > (r / 3);
                let hi_side = dx - dy < -r / 4;
                let mut base = if hi_side && h % 5 != 0 {
                    leaf[2]
                } else if shade_side {
                    leaf[3]
                } else if h % 3 == 0 {
                    leaf[1]
                } else {
                    leaf[0]
                };
                // leaf texture
                if s >= 2 && (dx + dy) % 3 == 0 {
                    base = sh(base, 0.92);
                }
                fb.put(bx + dx, cy + dy, mode.surf(base));
            }
        }
    }
}

/// Birch: pale trunk with bark rings, delicate airy foliage.
pub fn tree_birch(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, seed: u64, scale: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    ground_shadow(fb, g, tx, ty, 0.5 * scale, mode);
    let trunk = mode.surf(rgb(0xdf, 0xdc, 0xd0));
    let th = (4.5 * s as f32 * scale) as i32;
    for dy in 0..th {
        fb.put(bx, by - dy, trunk);
        if s >= 2 {
            fb.put(bx + 1, by - dy, trunk);
            if dy % 3 == 0 {
                fb.put(bx, by - dy, mode.surf(rgb(0x44, 0x40, 0x38))); // bark rings
            }
        }
    }
    let cy = by - (5.5 * s as f32 * scale) as i32;
    let r = ((3.5 * scale) as i32 * s).max(3);
    let leaf = [
        rgb(0x7c, 0xb8, 0x46), rgb(0x96, 0xd4, 0x5a), rgb(0xba, 0xee, 0x76), rgb(0x56, 0x94, 0x30),
    ];
    for dy in -r..=r {
        for dx in -(r * 6 / 5)..=(r * 6 / 5) {
            let h = col::hash2(seed as i32 + dx * 2, (seed / 3) as i32 + dy * 3);
            if h % 8 == 0 && s >= 2 {
                continue; // airy dappled gaps
            }
            let d = (dx * dx) as f32 / 1.35 + (dy * dy) as f32;
            if d <= (r * r) as f32 {
                let base = match h % 6 {
                    0..=1 => leaf[2], // sunlit pale green
                    2..=4 => leaf[1],
                    _ => leaf[3],
                };
                fb.put(bx + dx, cy + dy, mode.surf(base));
            }
        }
    }
}

/// Conifer: layered evergreen tiers with needle texture and rich shadows.
pub fn tree_pine(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, _seed: u64, scale: f32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let s = g.s;
    ground_shadow(fb, g, tx, ty, 0.6 * scale, mode);
    let tiers = 4 + ((scale - 0.5) as i32).max(0);
    let mut yy = by - s;
    let mut half = ((2.8 * scale) as i32 * s).max(3);
    let nd = [
        rgb(0x28, 0x62, 0x36), // base
        rgb(0x1c, 0x4a, 0x28), // shadow
        rgb(0x3a, 0x82, 0x48), // highlight
    ];
    fb.put(bx, by, mode.surf(c::TRUNK_DARK));
    for _ in 0..tiers {
        for dy in 0..(2 * s) {
            for dx in -half..=half {
                let lit = if dx < 0 && dy < s {
                    nd[2]
                } else if dx > 0 {
                    nd[1]
                } else {
                    nd[0]
                };
                fb.put(bx + dx, yy - dy, mode.surf(lit));
            }
        }
        yy -= 2 * s;
        half = (half - s).max(s);
    }
    fb.put(bx, yy, mode.surf(nd[2]));
}
