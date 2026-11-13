//! The showcase buildings (brief B25): clapboard house, brick main-street
//! shop, stucco townhouse, glass office, glass tower, plus cheap fillers for
//! background density. All procedural, all parameterized by Grid (32/64 px).

use render::palette::{self, Color, rgb};
use render::Framebuffer;

use crate::col::{self, Mode, c};
use crate::font;
use crate::geom::{Grid, Quad, box_faces, box_top};
use crate::parts::{self, WinOpts};

fn sh(c: Color, f: f32) -> Color {
    palette::shade(c, f)
}

/// Storey heights in px, at s=1 (s=2 doubles them).
pub const RES_FLOOR: i32 = 8;
pub const SHOP_GROUND: i32 = 10;
pub const SHOP_FLOOR: i32 = 8;
pub const TOWER_FLOOR: i32 = 6;

// ---------------------------------------------------------------------------
// Clapboard New England house with porch (2 floors + gable attic)
// ---------------------------------------------------------------------------

pub fn clapboard_house(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let (w, d) = (2.6f32, 2.2f32);
    let z1 = 2 * RES_FLOOR * s + 2 * s; // two storeys + floor slab
    let rise = 6 * s;

    // Directional isometric cast shadow
    parts::building_shadow(fb, g, x, y, x + w, y + d, z1 + rise, mode);

    // gable end walls (the two +x/-x triangles) first, walls over them
    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    // walls
    parts::paint_clap(&lf, fb, mode, c::CLAP, parts::FACE_L, s);
    parts::paint_clap(&rf, fb, mode, c::CLAP_SHADE, parts::FACE_R, s);
    // corner boards + water table
    edge_trim(fb, g, mode, x, y, x + w, y + d, z1, c::TRIM, s);

    // gable ends (fill triangles above the wall tops at x and x+w)
    let my = y + d / 2.0;
    let tri_r = [
        g.p(x + w, y, z1),
        g.p(x + w, y + d, z1),
        g.p(x + w, my, z1 + rise),
    ];
    fb.fill_poly(&tri_r, mode.surf(sh(c::CLAP_SHADE, parts::FACE_R)));
    let tri_l = [
        g.p(x, y, z1),
        g.p(x, y + d, z1),
        g.p(x, my, z1 + rise),
    ];
    fb.fill_poly(&tri_l, mode.surf(c::CLAP));
    // outline the gable edges
    let (ax, ay) = g.pi_f(x + w, y, z1);
    let (bx, by) = g.pi_f(x + w, my, z1 + rise);
    let (cx, cy) = g.pi_f(x + w, y + d, z1);
    let edge = mode.surf(sh(c::ROOF_SHINGLE_DARK, 0.8));
    fb.line((ax as f32, ay as f32), (bx as f32, by as f32), edge);
    fb.line((bx as f32, by as f32), (cx as f32, cy as f32), edge);

    // roof over both, overhang 0.15 tile
    parts::roof_gable(
        fb, g, mode,
        x - 0.15, y - 0.15, x + w + 0.15, y + d + 0.15,
        z1, rise, c::ROOF_SHINGLE,
    );

    // windows: two per floor on right face, two upper floor on left face
    let wo = WinOpts {
        w: 4 * s,
        h: 4 * s,
        frame: c::TRIM,
        sill: true,
        shutters: true,
        shutter_c: c::SHUTTER,
        seed,
        lit_p: 0.72,
    };
    parts::window_grid(&rf, fb, mode, 2, 2, 3 * s, 2 * s, RES_FLOOR * s, wo, s);

    // upper windows on front (left) face
    let mut wo_front = wo;
    wo_front.seed = seed + 42;
    parts::window_grid(&lf, fb, mode, 2, 1, 3 * s, 2 * s, RES_FLOOR * s, wo_front, s);

    // front door on the left face, ground floor
    let rows = lf.rows() as f32;
    let dw = 4 * s;
    let du = lf.top_len() / 2 - dw / 2;
    let dq = lf.sub_px(du, rows as i32 - 6 * s, du + dw, rows as i32, rows);
    dq.paint(fb, &mut |u, v, _, _| {
        if u == 0 || v == 0 || u == dw - 1 {
            mode.surf(c::TRIM)
        } else if s >= 2 && u == dw / 2 && v > 2 {
            mode.surf(sh(c::DOOR, 1.3))
        } else {
            mode.surf(c::DOOR)
        }
    });
    // brass knob
    {
        let (kx, ky) = dq.pt(0.75, 0.65, dq.rows() as f32);
        fb.put(kx as i32, ky as i32, mode.surf(rgb(0xd8, 0xb8, 0x4c)));
    }
    // porch lantern beside the door
    {
        let (lx, ly) = dq.pt(1.15, 0.35, dq.rows() as f32);
        let lamp_col = match mode {
            Mode::Day => mode.surf(c::LAMP_POST),
            Mode::Night => {
                parts::glow(fb, lx as i32, ly as i32, 5 * s, 3 * s, 0.6, col::LAMP);
                col::LAMP
            }
        };
        fb.put(lx as i32, ly as i32, lamp_col);
    }

    // Front porch spanning the front entrance
    porch_front(fb, g, mode, x + 0.3, y + d, x + w - 0.3, y + d + 0.55, seed);
    chimney(fb, g, mode, x + w * 0.3, my, z1 + rise - 2, 5 * s, seed);
}

/// Front porch on the front (+y) face: plank deck, white columns, railing, shed roof, steps.
fn porch_front(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, y0: f32, x1: f32, y1: f32, _seed: u64) {
    let s = g.s;
    let slab_h = 2 * s;
    let roof_z = 7 * s + slab_h;

    // Deck slab
    let deck = box_top(g, x0, y0, x1, y1, slab_h);
    deck.paint(fb, &mut |_u, v, _, _| {
        let plank = (v % (2 * s)) == 0;
        mode.surf(sh(c::PORCH_BOARDS, if plank { 0.90 } else { 1.0 }))
    });

    // Deck front face
    let (dlf, _drf) = box_faces(g, x0, y0, x1, y1, 0, slab_h);
    dlf.fill(fb, mode.surf(sh(c::TRIM, 0.85)));

    // White turned columns at front corners and center
    let cols = [x0 + 0.05, (x0 + x1) / 2.0, x1 - 0.05];
    for &cx in &cols {
        let (px, py) = g.pi_f(cx, y1 - 0.05, slab_h);
        for dy in 0..(roof_z - slab_h) {
            fb.put(px, py - dy, mode.surf(c::TRIM));
            if s >= 2 {
                fb.put(px + 1, py - dy, mode.surf(sh(c::TRIM, 0.92)));
            }
        }
    }

    // Balustrade railing between columns
    for (&ca, &cb) in cols.iter().zip(cols.iter().skip(1)) {
        let (ax, ay) = g.pi_f(ca, y1 - 0.05, slab_h);
        let (bx, by) = g.pi_f(cb, y1 - 0.05, slab_h);
        let steps = ((bx - ax).abs().max((by - ay).abs())).max(1);
        let rail_h = 2 * s;
        for i in 0..=steps {
            let px = ax + (bx - ax) * i / steps;
            let py = ay + (by - ay) * i / steps;
            // Leave a gap for the center entrance opening
            if ca == cols[0] && i > steps * 3 / 4 {
                continue; // entrance opening
            }
            if ca == cols[1] && i < steps / 4 {
                continue; // entrance opening
            }
            if i % (s.max(1)) == 0 {
                for dy in 0..rail_h {
                    fb.put(px, py - dy, mode.surf(c::TRIM));
                }
            } else {
                fb.put(px, py - rail_h, mode.surf(c::TRIM)); // top rail
            }
        }
    }

    // Sloped porch roof
    let prq = crate::geom::corridor_quad(g, x0 - 0.08, y0, x1 + 0.08, y1 + 0.12, roof_z);
    prq.paint(fb, &mut |_, _, _, _| mode.surf(sh(c::ROOF_SHINGLE, 1.04)));
    // Fascia trim along the front edge of the porch roof
    fb.line(prq.bl, prq.br, mode.surf(c::TRIM));

    // Front steps descending from the porch opening
    let (mx, my) = ((x0 + x1) / 2.0, y1);
    for st in 0..slab_h {
        let sq = crate::geom::corridor_quad(g, mx - 0.25, my + st as f32 * 0.12, mx + 0.25, my + (st + 1) as f32 * 0.12, slab_h - st);
        sq.fill(fb, mode.surf(sh(c::CONCRETE, 0.95 - 0.05 * st as f32)));
    }
}

/// Brick chimney with a clay pot.
fn chimney(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, z: i32, h: i32, _s: u64) {
    let (lf, rf) = box_faces(g, tx, ty, tx + 0.25, ty + 0.25, z - h, z);
    lf.fill(fb, mode.surf(c::BRICK));
    rf.fill(fb, mode.surf(sh(c::BRICK, crate::parts::FACE_R)));
    let tq = box_top(g, tx, ty, tx + 0.25, ty + 0.25, z + g.s);
    tq.fill(fb, mode.surf(sh(c::CONCRETE, 0.9)));
}

/// Corner boards + a water table along the base of a clapboard box.
#[allow(clippy::too_many_arguments)]
fn edge_trim(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, y0: f32, x1: f32, y1: f32, z1: i32, trim: Color, s: i32) {
    // vertical corner board at the front corner (x1,y1)
    let (px, py) = g.pi_f(x1, y1, z1);
    for dy in 0..z1 {
        fb.put(px, py + dy, mode.surf(trim));
        if s >= 2 {
            fb.put(px + 1, py + dy, mode.surf(sh(trim, 0.92)));
        }
    }
    // water table: a 1-2px light line along both faces' base
    let (blf, brf) = box_faces(g, x0, y0, x1, y1, 0, s.max(1));
    blf.fill(fb, mode.surf(sh(trim, 0.85)));
    brf.fill(fb, mode.surf(sh(trim, 0.75)));
}

// ---------------------------------------------------------------------------
// Brick 3-storey main-street shop with awning, sign, neon + billboard
// ---------------------------------------------------------------------------

pub fn brick_shop(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let (w, d) = (2.4f32, 2.2f32);
    let z_g = SHOP_GROUND * s;
    let z1 = z_g + 2 * SHOP_FLOOR * s;

    // Directional building cast shadow
    parts::building_shadow(fb, g, x, y, x + w, y + d, z1, mode);

    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    // upper floors brick; ground floor painted piers
    let rows = lf.rows();
    let gfrac_v = |face: &Quad| face.sub_px(0, rows - z_g, face.top_len(), rows, rows as f32);
    let lf_up = lf.sub_px(0, 0, lf.top_len(), rows - z_g, rows as f32);
    let rf_up = rf.sub_px(0, 0, rf.top_len(), rows - z_g, rows as f32);
    parts::paint_brick(&lf_up, fb, mode, c::BRICK, parts::FACE_L, s);
    parts::paint_brick(&rf_up, fb, mode, sh(c::BRICK, 0.92), parts::FACE_R, s);
    parts::paint_flat(&gfrac_v(&lf), fb, mode, sh(c::BRICK, 0.75), parts::FACE_L);
    parts::paint_flat(&gfrac_v(&rf), fb, mode, sh(c::BRICK, 0.68), parts::FACE_R);

    // storefront: big panes + recessed door on the left face
    storefront(fb, g, mode, &lf, s, seed);

    // awning over the sidewalk above the storefront display windows
    awning(fb, g, mode, &lf, rows - z_g + 2 * s, c::AWNING_A, c::AWNING_B);

    // sign band above the awning
    sign_band(fb, g, mode, &lf, rows - z_g - s, "CAFE", parts::FACE_L, s);

    // upper windows: double-hung with lintels + sills
    let wo = WinOpts {
        w: 4 * s,
        h: 4 * s,
        frame: c::TRIM,
        sill: true,
        shutters: false,
        shutter_c: c::TRIM,
        seed: seed + 11,
        lit_p: 0.45,
    };
    let mut wo_r = wo;
    wo_r.seed = seed + 19;
    upper_windows_shop(&lf_up, fb, mode, wo, s, parts::FACE_L);
    upper_windows_shop(&rf_up, fb, mode, wo_r, s, parts::FACE_R);

    // cornice + parapet + roof
    cornice(fb, g, mode, x, y, x + w, y + d, z1, s);
    parts::roof_flat(fb, g, mode, x + 0.15, y + 0.15, x + w - 0.15, y + d - 0.15, z1 + 2 * s,
        sh(c::CONCRETE, 0.92), c::BRICK_MORTAR, s >= 2, seed);

    // rooftop billboard + neon corner sign (lit at night, D55)
    billboard(fb, g, mode, x + 0.6, y + 0.4, z1 + 2 * s, s);
    neon_sign(fb, g, mode, x + w, y + d - 0.35, 2 * s, z1 - 4 * s, s);

    let _ = wo;
}

/// Ground-floor storefront: pane glass, door, kick plates.
fn storefront(fb: &mut Framebuffer, _g: &Grid, mode: Mode, lf: &Quad, s: i32, seed: u64) {
    let rows = lf.rows();
    let fw = lf.top_len();
    let glass_h = 6 * s;
    let v0 = rows - glass_h - 2 * s; // leave a kick plate + threshold
    let panes = 3;
    let margin = 2 * s;
    let step = (fw - 2 * margin) / (panes + 1); // + door takes the last slot
    for i in 0..panes {
        let u0 = margin + i * step;
        let u1 = margin + (i + 1) * step - s;
        let pq = lf.sub_px(u0, v0, u1, rows - 2 * s, rows as f32);
        let litp = 0.8;
        let lit = (col_hash(seed, i as u64) % 100) < (litp * 100.0) as u64;
        let glass = match mode {
            Mode::Day => sh(c::GLASS_B, 0.85),
            Mode::Night => {
                if lit { rgb(0xff, 0xd8, 0x92) } else { sh(col::WIN_OFF, 0.8) }
            }
        };
        pq.paint(fb, &mut |u, v, _, _| {
            if u == 0 || v == 0 || u == (u1 - u0) - 1 {
                mode.surf(c::MULLION)
            } else if mode == Mode::Day && (u + v) % (5 * s) < s {
                sh(glass, 1.35) // glint
            } else {
                glass
            }
        });
        // kick plate
        let kq = lf.sub_px(u0, rows - 2 * s, u1, rows, rows as f32);
        kq.fill(fb, mode.surf(sh(c::MULLION, 0.8)));
    }
    // door: last slot
    let du0 = margin + panes * step;
    let dq = lf.sub_px(du0, v0, du0 + step - s, rows, rows as f32);
    dq.paint(fb, &mut |u, v, _, _| {
        if u < s || v < s {
            mode.surf(c::MULLION)
        } else if mode == Mode::Night {
            rgb(0xff, 0xcf, 0x86)
        } else {
            sh(c::GLASS_B, 1.05)
        }
    });
}

#[inline]
fn fh_none() {}
#[inline]
fn col_hash(seed: u64, i: u64) -> u64 {
    let mut h = seed.wrapping_mul(0x9E3779B97F4A7C15) ^ i.wrapping_mul(0xC2B2AE3D27D4EB4F);
    h ^= h >> 29;
    h.wrapping_mul(0x165667B19E3779F9)
}

/// Striped awning sloping down over the sidewalk.
fn awning(fb: &mut Framebuffer, g: &Grid, mode: Mode, face: &Quad, top_v: i32, ca: Color, cb: Color) {
    let s = g.s;
    let rows = face.rows();
    let out = (2 * s).max(2); // slope drop in px
    let top_a = face.pt(0.04, (top_v - 2 * s) as f32, rows as f32);
    let top_b = face.pt(0.96, (top_v - 2 * s) as f32, rows as f32);
    let dir = if top_a.0 < top_b.0 { (-0.30f32, 0.18f32) } else { (0.30, 0.18) };
    let span = g.tw as f32 * 0.22;
    let a2 = (top_a.0 + dir.0 * span, top_a.1 + out as f32 + dir.1 * span);
    let b2 = (top_b.0 + dir.0 * span, top_b.1 + out as f32 + dir.1 * span);
    let aq = Quad::new(top_a, top_b, b2, a2);
    let stripe = (3 * s).max(2);
    aq.paint(fb, &mut |u, v, _, _| {
        let f = if v == aq.rows() - 1 { 0.80 } else { 1.0 };
        let st = if (u / stripe) % 2 == 0 { ca } else { cb };
        mode.surf(sh(st, f))
    });
    // Scalloped valance edge hanging down
    let (v_lx, v_ly) = a2;
    let (v_rx, v_ry) = b2;
    let steps = ((v_rx - v_lx).abs().max((v_ry - v_ly).abs())).max(1.0) as i32;
    for i in 0..=steps {
        let px = (v_lx + (v_rx - v_lx) * i as f32 / steps as f32).round() as i32;
        let py = (v_ly + (v_ry - v_ly) * i as f32 / steps as f32).round() as i32;
        let st = if (i / stripe) % 2 == 0 { ca } else { cb };
        fb.put(px, py, mode.surf(sh(st, 0.88)));
        if s >= 2 {
            fb.put(px, py + 1, mode.surf(sh(st, 0.78)));
        }
    }
}

/// A dark sign band with text along the face's diagonal.
fn sign_band(fb: &mut Framebuffer, _g: &Grid, mode: Mode, face: &Quad, top_v: i32, text: &str, _side: f32, s: i32) {
    let rows = face.rows();
    let hh = 6 * s;
    let bq = face.sub_px(s, top_v - hh, face.top_len() - s, top_v, rows as f32);
    bq.fill(fb, mode.surf(c::SHOP_SIGN));
    // gold pinstripe
    let pq = bq.sub_px(0, hh - s, bq.top_len(), hh, bq.rows() as f32);
    pq.fill(fb, mode.surf(rgb(0xc8, 0xa8, 0x54)));
    // text following the band's slope (iso faces fall 1 px per 2 px of x)
    let bb = crate::geom::quad_bounds(&bq);
    let tw = font::text_w(text, s);
    let x0 = bb.x0 + (bb.width() - tw) / 2;
    let y0 = bb.y0 + (hh - 5 * s) / 2 + s;
    font::text_slope(fb, x0, y0, text, mode.surf(rgb(0xee, 0xe6, 0xd2)), s, 1);
}

/// Upper-floor windows with stone lintels for the shop.
fn upper_windows_shop(up: &Quad, fb: &mut Framebuffer, mode: Mode, o: WinOpts, s: i32, side: f32) {
    let rows = up.rows();
    let floors = 2;
    let floor_h = rows / floors;
    let cols = (up.top_len() / (8 * s)).max(2);
    let fw = up.top_len();
    for f in 0..floors {
        for cx in 0..cols {
            let step = fw / cols;
            let u = cx * step + (step - o.w) / 2;
            let v = f * floor_h + (floor_h - o.h) / 2 + s;
            parts::window(up, fb, mode, u, v, o, s);
            // lintel
            let lq = up.sub_px(u - s, v - s, u + o.w + s, v, rows as f32);
            lq.fill(fb, mode.surf(sh(c::TRIM, 0.8 * side)));
        }
    }
}

/// Cornice band at the top of the shop fronts.
fn cornice(fb: &mut Framebuffer, g: &Grid, mode: Mode, x0: f32, y0: f32, x1: f32, y1: f32, z: i32, s: i32) {
    let (lf, rf) = box_faces(g, x0 - 0.06, y0 - 0.06, x1 + 0.06, y1 + 0.06, z - s, z + s);
    lf.fill(fb, mode.surf(c::TRIM));
    rf.fill(fb, mode.surf(sh(c::TRIM, parts::FACE_R)));
    let _ = g;
}

/// Rooftop billboard (D55): steel lattice frame, colorful advertising poster, lit by floodlights at night.
fn billboard(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, z: i32, s: i32) {
    let w = 12 * s;
    let h = 6 * s;
    let (bx, by) = g.pi_f(tx, ty, z);
    // Steel lattice support posts
    for i in 0..=2 {
        let px = bx + i * (w - 2 * s) / 2;
        for dy in 0..3 * s {
            fb.put(px, by - dy, mode.surf(c::MULLION));
            if s >= 2 {
                fb.put(px + 1, by - dy, mode.surf(sh(c::MULLION, 0.85)));
            }
        }
    }
    // Panel angled along the boulevard (+y face)
    let top = by - 3 * s - h;
    let panel = Quad::new(
        (bx as f32, top as f32),
        (bx as f32 + w as f32, (top + w / 4) as f32),
        (bx as f32 + w as f32, (by - 3 * s + w / 4) as f32),
        (bx as f32, (by - 3 * s) as f32),
    );
    match mode {
        Mode::Day => {
            panel.paint(fb, &mut |u, v, _, _| {
                if u < s || v < s || u >= w - s || v >= h - s {
                    mode.surf(c::MULLION) // steel frame border
                } else {
                    let pos = ((u - w / 2) * (u - w / 2) / 4 + (v - h / 2) * (v - h / 2)) as i32;
                    if pos < (2 * s) * (2 * s) {
                        rgb(0xe8, 0x5a, 0x44) // coffee cup / sunrise
                    } else if v < h / 2 {
                        rgb(0xf2, 0xe8, 0xd0) // bright cream top
                    } else {
                        rgb(0x4a, 0x8a, 0xc4) // blue sky / graphic band
                    }
                }
            });
        }
        Mode::Night => {
            // Bright lit face + floodlight spill
            panel.paint(fb, &mut |u, v, _, _| {
                if u < s || v < s || u >= w - s || v >= h - s {
                    rgb(0x18, 0x20, 0x2c)
                } else {
                    let pos = ((u - w / 2) * (u - w / 2) / 4 + (v - h / 2) * (v - h / 2)) as i32;
                    if pos < (2 * s) * (2 * s) {
                        col::NEON_ORANGE
                    } else if v < h / 2 {
                        rgb(0xff, 0xf6, 0xde)
                    } else {
                        rgb(0x6a, 0xaa, 0xe4)
                    }
                }
            });
            // Overhead floodlight wash on the billboard and roof
            parts::glow(fb, bx + w / 2, top + h / 2, 10 * s, 6 * s, 0.45, rgb(0xff, 0xf4, 0xd8));
            parts::glow(fb, bx + w / 2, by, 8 * s, 3 * s, 0.30, rgb(0xff, 0xea, 0xcc));
        }
    }
}

/// Vertical neon sign on the shop's street corner; on at night (D55).
fn neon_sign(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, z0: i32, z1: i32, s: i32) {
    let (bx, by) = g.pi_f(tx, ty, 0);
    let x0 = bx + 2;
    let y0 = by - z1;
    let h = z1 - z0;
    let w = 3 * s;

    // Metal mounting brackets
    for &dy in &[y0 + 2 * s, y0 + h - 2 * s] {
        fb.put(bx, dy, mode.surf(c::LAMP_POST));
        fb.put(bx + 1, dy, mode.surf(c::LAMP_POST));
    }

    match mode {
        Mode::Day => {
            for yy in y0..(y0 + h) {
                for xx in x0..(x0 + w) {
                    let border = xx == x0 || xx == x0 + w - 1 || yy == y0 || yy == y0 + h - 1;
                    fb.put(xx, yy, mode.surf(if border { c::TRIM } else { sh(c::SHOP_SIGN, 1.1) }));
                }
            }
            // Muted neon tube outline
            for yy in (y0 + 2)..=(y0 + h - 2) {
                fb.put(x0 + w / 2, yy, mode.surf(sh(col::NEON_PINK, 0.7)));
            }
        }
        Mode::Night => {
            // Atmospheric neon glow casting onto wall and sidewalk below (D55 + Ref 9)
            parts::glow(fb, x0 + w / 2, y0 + h / 2, 14 * s, 8 * s, 0.45, col::NEON_PINK);
            parts::glow(fb, x0 + w / 2, y0 + h / 2, 8 * s, 5 * s, 0.55, col::NEON_CYAN);
            // Sidewalk reflection below
            parts::glow(fb, bx, by, 10 * s, 5 * s, 0.35, col::NEON_PINK);

            // Dark backing panel
            for yy in y0..(y0 + h) {
                for xx in x0..(x0 + w) {
                    fb.put(xx, yy, rgb(0x18, 0x10, 0x1a));
                }
            }
            // Vertical letters in glowing neon
            let letters = ['C', 'A', 'F', 'E'];
            let lh = h / letters.len() as i32;
            for (i, ch) in letters.iter().enumerate() {
                let ly = y0 + i as i32 * lh + (lh - 5) / 2;
                font::text(fb, x0 + (w - 3) / 2, ly, &ch.to_string(), rgb(0xff, 0xff, 0xff), 1);
            }
            // Vibrant neon tube borders
            for yy in y0..(y0 + h) {
                fb.put(x0, yy, col::NEON_PINK);
                fb.put(x0 + w - 1, yy, col::NEON_CYAN);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// European stucco townhouse: 4 floors, tall windows, balconies, tile hip roof
// ---------------------------------------------------------------------------

pub fn stucco_townhouse(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let (w, d) = (2.2f32, 2.0f32);
    let floors = 4;
    let z1 = floors * RES_FLOOR * s;
    let rise = 5 * s;

    // Directional building cast shadow
    parts::building_shadow(fb, g, x, y, x + w, y + d, z1 + rise, mode);

    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    parts::paint_stucco(&lf, fb, mode, c::STUCCO, parts::FACE_L, s);
    parts::paint_stucco(&rf, fb, mode, c::STUCCO_SHADE, parts::FACE_R, s);

    // quoins (pale corner stones) on the corners
    if s >= 1 {
        let (px, py) = g.pi_f(x + w, y + d, z1);
        for dy in 0..z1 {
            if (dy / (2 * s.max(1))) % 2 == 0 {
                fb.put(px, py + dy, mode.surf(c::TRIM));
                if s >= 2 {
                    fb.put(px - 1, py + dy, mode.surf(sh(c::TRIM, 0.95)));
                }
            }
        }
    }

    // string courses between floors
    for f in 1..floors {
        let v = f * RES_FLOOR * s;
        let (lq, rq) = (
            lf.sub_px(0, v - s, lf.top_len(), v, lf.rows() as f32),
            rf.sub_px(0, v - s, rf.top_len(), v, rf.rows() as f32),
        );
        lq.fill(fb, mode.surf(sh(c::TRIM, 0.95)));
        rq.fill(fb, mode.surf(sh(c::TRIM, 0.8)));
    }

    // tall windows, balconies with flower boxes on floors 2..4
    let wo = WinOpts {
        w: 3 * s,
        h: 5 * s,
        frame: sh(c::TRIM, 0.9),
        sill: true,
        shutters: false,
        shutter_c: c::TRIM,
        seed: seed + 5,
        lit_p: 0.55,
    };
    for f in 0..floors {
        let floor_h = RES_FLOOR * s;
        let v = f * floor_h + (floor_h - wo.h) / 2;
        for (face, side) in [(&lf, parts::FACE_L), (&rf, parts::FACE_R)] {
            let fw = face.top_len();
            let cols = (fw / (7 * s)).max(2);
            let step = fw / cols;
            for cx in 0..cols {
                let u = cx * step + (step - wo.w) / 2;
                let mut oo = wo;
                oo.seed = wo.seed + (f * 41 + cx * 7) as u64 + (side * 100.0) as u64;
                parts::window(face, fb, mode, u, v, oo, s);
                if f < floors - 1 && s >= 1 {
                    balcony(face, fb, mode, u - s, v + wo.h, wo.w + 2 * s, s);
                }
            }
        }
    }

    // ground door with stone arch + double carriage doors on the left face
    let rows = lf.rows();
    let dw = 4 * s;
    let du = lf.top_len() / 2 - dw / 2;
    let dq = lf.sub_px(du, rows - 6 * s, du + dw, rows, rows as f32);
    dq.paint(fb, &mut |u, v, _, _| {
        if v < s || u == 0 || u == dw - 1 {
            mode.surf(c::TRIM) // stone arch surround
        } else if s >= 2 && u == dw / 2 {
            mode.surf(rgb(0x28, 0x1c, 0x14)) // door seam
        } else {
            mode.surf(c::DOOR)
        }
    });

    // roof
    parts::roof_hip_tile(fb, g, mode, x - 0.12, y - 0.12, x + w + 0.12, y + d + 0.12, z1, rise);
    // Rooftop dormer window on the front roof slope
    dormer(fb, g, mode, x + w * 0.45, y + d - 0.25, z1, 4 * s, s);
    chimney(fb, g, mode, x + w * 0.65, y + d * 0.35, z1 + rise, 4 * s, seed);
}

/// A dormer window set into the tile roof slope.
fn dormer(fb: &mut Framebuffer, g: &Grid, mode: Mode, tx: f32, ty: f32, z: i32, dh: i32, s: i32) {
    let (w, d) = (0.5f32, 0.35f32);
    let (dlf, _drf) = box_faces(g, tx, ty, tx + w, ty + d, z, z + dh);
    dlf.fill(fb, mode.surf(c::STUCCO));
    let dwq = dlf.sub_px(s, s, dlf.top_len() - s, dh - s, dlf.rows() as f32);
    dwq.fill(fb, mode.surf(sh(c::GLASS_HI, 0.9)));
    let dt = box_top(g, tx - 0.05, ty - 0.05, tx + w + 0.05, ty + d + 0.05, z + dh);
    dt.fill(fb, mode.surf(c::TILE_ROOF_DARK));
}

/// A tiny wrought-iron balcony under a window, with blooming flower box.
fn balcony(face: &Quad, fb: &mut Framebuffer, mode: Mode, u0: i32, v0: i32, wpx: i32, s: i32) {
    let rows = face.rows();
    // stone slab bracket
    let sq = face.sub_px(u0, v0, u0 + wpx, v0 + s, rows as f32);
    sq.fill(fb, mode.surf(sh(c::PARAPET, 0.9)));
    // railing: vertical balusters
    let rq = face.sub_px(u0, v0 + s, u0 + wpx, v0 + 3 * s, rows as f32);
    rq.paint(fb, &mut |u, v, _, _| {
        if v == 0 {
            mode.surf(c::RAIL)
        } else if u % (2 * s).max(2) == 0 {
            mode.surf(sh(c::RAIL, 0.85))
        } else if v == 3 * s - 1 {
            mode.surf(sh(c::RAIL, 0.7))
        } else {
            palette::TRANSPARENT
        }
    });
    // Blooming flower box on the balcony railing
    if s >= 2 {
        let fq = face.sub_px(u0 + 1, v0 + 2 * s, u0 + wpx - 1, v0 + 3 * s, rows as f32);
        fq.paint(fb, &mut |u, _v, _, _| {
            let flower = match u % 3 {
                0 => rgb(0xe8, 0x24, 0x3a), // scarlet geranium
                1 => rgb(0xf2, 0x64, 0x8a), // pink geranium
                _ => rgb(0x3e, 0x84, 0x34), // green foliage
            };
            mode.surf(flower)
        });
    }
}

// ---------------------------------------------------------------------------
// Glass office block (8+ floors)
// ---------------------------------------------------------------------------

pub fn glass_office(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, floors: i32, seed: u64) {
    let s = g.s;
    let (w, d) = (2.6f32, 2.4f32);
    let lobby_h = SHOP_GROUND * s;
    let z1 = lobby_h + floors * TOWER_FLOOR * s;

    // Directional building cast shadow
    parts::building_shadow(fb, g, x, y, x + w, y + d, z1, mode);

    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    let rows = lf.rows();
    let lf_up = lf.sub_px(0, 0, lf.top_len(), rows - lobby_h, rows as f32);
    let rf_up = rf.sub_px(0, 0, rf.top_len(), rows - lobby_h, rows as f32);
    curtain(&lf_up, fb, mode, 4 * s, parts::FACE_L, seed, 0.35);
    curtain(&rf_up, fb, mode, 4 * s, parts::FACE_R, seed + 3, 0.35);

    // Lobby: double-height glass with structural columns and interior light
    let lq = lf.sub_px(0, rows - lobby_h, lf.top_len(), rows, rows as f32);
    let rq = rf.sub_px(0, rows - lobby_h, rf.top_len(), rows, rows as f32);
    let lg = match mode {
        Mode::Day => sh(c::GLASS_B, 0.85),
        Mode::Night => rgb(0xff, 0xe2, 0xa8),
    };
    lq.paint(fb, &mut |u, v, _, _| {
        if u % (8 * s) == 0 || v < s {
            mode.surf(c::MULLION)
        } else {
            sh(lg, 0.96)
        }
    });
    rq.paint(fb, &mut |u, v, _, _| {
        if u % (8 * s) == 0 || v < s {
            mode.surf(c::MULLION)
        } else {
            sh(lg, 0.76)
        }
    });
    // Canopy over the front entrance
    let cq = crate::geom::corridor_quad(g, x + 0.4, y + d, x + 1.8, y + d + 0.45, lobby_h - 2 * s);
    cq.fill(fb, mode.surf(c::MULLION));

    // Roof: parapet + penthouse elevator room + chillers
    parts::roof_flat(fb, g, mode, x, y, x + w, y + d, z1, sh(c::CONCRETE, 0.85), c::PARAPET, true, seed);
    let (plf, prf) = box_faces(g, x + 0.5, y + 0.4, x + 1.4, y + 1.2, z1, z1 + 3 * s);
    plf.fill(fb, mode.surf(c::CONCRETE));
    prf.fill(fb, mode.surf(sh(c::CONCRETE, parts::FACE_R)));
    let ptop = box_top(g, x + 0.5, y + 0.4, x + 1.4, y + 1.2, z1 + 3 * s);
    ptop.fill(fb, mode.surf(sh(c::CONCRETE, 0.85)));
}

/// Curtain-wall glass: mullion grid, vertical gradient, diagonal reflections,
/// floor-slab spandrels, coherent lit office rooms at night.
fn curtain(face: &Quad, fb: &mut Framebuffer, mode: Mode, cell: i32, side: f32, seed: u64, lit_p: f32) {
    let fh = (cell * 3 / 2).max(3); // floor band height in px
    let rows = face.rows();
    let flen = face.top_len();
    face.paint(fb, &mut |u, v, _x, _y| {
        let mullion = (u % cell == 0) || (u == flen - 1);
        let slab = (v % fh == 0) || (v == rows - 1);
        if slab {
            return mode.surf(sh(c::PARAPET, side * 1.15));
        }
        if mullion {
            return mode.surf(sh(c::MULLION, side));
        }
        match mode {
            Mode::Day => {
                // Smooth vertical sky reflection gradient
                let gpos = v as f32 / rows as f32;
                let mut gc = palette::mix(c::GLASS_A, c::GLASS_B, gpos);
                // Broad diagonal cloud / horizon reflection band across the facade
                let diag = (u * 2 + v * 3) / (2 * cell);
                if diag % 8 == 2 || diag % 8 == 3 {
                    gc = palette::mix(gc, c::GLASS_HI, 0.55);
                } else if diag % 8 == 4 {
                    gc = palette::mix(gc, c::GLASS_HI, 0.25);
                }
                // Subtle floor spandrel shadow at top of each floor
                if (v % fh) == 1 {
                    gc = sh(gc, 0.85);
                }
                mode.surf(sh(gc, side))
            }
            Mode::Night => {
                // Office clustering: rooms/floors are lit together
                let room_u = u / (cell * 2).max(2);
                let floor_v = v / fh.max(1);
                let h = col_hash(seed, (room_u * 37 + floor_v * 199) as u64);
                let lit = (h % 1000) as f32 / 1000.0;
                if lit < lit_p {
                    if (h % 5) == 0 {
                        col::WIN_COOL
                    } else {
                        col::WIN_WARM
                    }
                } else {
                    sh(sh(c::GLASS_B, 0.32), side)
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Glass tower, 20+ floors, setback, crown, beacon (D55)
// ---------------------------------------------------------------------------

pub fn glass_tower(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let fl = TOWER_FLOOR * s;
    let (w, d) = (3.2f32, 3.0f32);
    let pod_h = 3 * 8 * s; // 3 floors podium
    let shaft_h = 14 * fl; // 14 floors shaft
    let up_h = 7 * fl; // 7 floors upper (total 24 floors!)
    let z_top = pod_h + shaft_h + up_h;

    // Directional building shadow on the ground
    parts::building_shadow(fb, g, x, y, x + w, y + d, z_top + 10 * s, mode);

    // Shaft faces
    let (slf, srf) = box_faces(g, x, y, x + w, y + d, pod_h, pod_h + shaft_h);
    curtain(&slf, fb, mode, 4 * s, parts::FACE_L, seed + 1, 0.36);
    curtain(&srf, fb, mode, 4 * s, parts::FACE_R, seed + 2, 0.36);

    // Podium faces (granite & glass entrance atrium)
    let (plf, prf) = box_faces(g, x - 0.2, y - 0.2, x + w + 0.2, y + d + 0.2, 0, pod_h);
    podium(&plf, fb, mode, parts::FACE_L, s, seed);
    podium(&prf, fb, mode, parts::FACE_R, s, seed + 1);

    // Setback terrace at floor 17
    let tq = crate::geom::corridor_quad(g, x, y, x + w, y + d, pod_h + shaft_h);
    tq.paint(fb, &mut |u, v, _, _| {
        let jit = 0.92 + 0.08 * col::hash01(u, v);
        mode.surf(sh(c::CONCRETE, 0.88 * jit))
    });

    // Planters on the terrace deck
    let plq = crate::geom::corridor_quad(g, x + 0.2, y + d - 0.3, x + w - 0.2, y + d - 0.1, pod_h + shaft_h);
    plq.fill(fb, mode.surf(c::HEDGE));

    // Terrace perimeter glass balustrade
    let (trl, trr) = box_faces(g, x, y, x + w, y + d, pod_h + shaft_h, pod_h + shaft_h + 2 * s);
    trl.fill(fb, mode.surf(sh(c::GLASS_HI, 0.6)));
    trr.fill(fb, mode.surf(sh(c::GLASS_HI, 0.5)));

    // Upper setback tower (floors 18..24)
    let ux = x + 0.4;
    let uy = y + 0.4;
    let (ulf, urf) = box_faces(g, ux, uy, x + w - 0.4, y + d - 0.4, pod_h + shaft_h, z_top);
    curtain(&ulf, fb, mode, 4 * s, parts::FACE_L, seed + 5, 0.32);
    curtain(&urf, fb, mode, 4 * s, parts::FACE_R, seed + 6, 0.32);

    // Upper roof deck
    let top_deck = box_top(g, ux, uy, x + w - 0.4, y + d - 0.4, z_top);
    top_deck.fill(fb, mode.surf(sh(c::CONCRETE, 0.85)));

    // Crown Tier 1: louvered mechanical penthouse
    let c1 = z_top + 4 * s;
    let (c1lf, c1rf) = box_faces(g, ux + 0.3, uy + 0.3, x + w - 0.7, y + d - 0.7, z_top, c1);
    let crown_col = match mode {
        Mode::Day => sh(c::STEEL, 1.15),
        Mode::Night => rgb(0x28, 0x3e, 0x5a),
    };
    c1lf.fill(fb, mode.surf(crown_col));
    c1rf.fill(fb, mode.surf(sh(crown_col, parts::FACE_R)));
    let c1top = box_top(g, ux + 0.3, uy + 0.3, x + w - 0.7, y + d - 0.7, c1);
    c1top.fill(fb, mode.surf(sh(crown_col, 1.1)));

    // Crown Tier 2: open architectural crown frame / fin
    let c2 = c1 + 4 * s;
    let (c2lf, c2rf) = box_faces(g, ux + 0.6, uy + 0.6, x + w - 1.0, y + d - 1.0, c1, c2);
    c2lf.fill(fb, mode.surf(sh(crown_col, 1.2)));
    c2rf.fill(fb, mode.surf(sh(crown_col, 1.2 * parts::FACE_R)));
    let c2top = box_top(g, ux + 0.6, uy + 0.6, x + w - 1.0, y + d - 1.0, c2);
    c2top.fill(fb, mode.surf(sh(crown_col, 1.25)));

    // Spire / communications mast reaching high above the crown
    let (spx, spy) = g.pi_f((ux + x + w - 0.4) / 2.0, (uy + y + d - 0.4) / 2.0, c2);
    let spire_h = 12 * s;
    for dy in 0..spire_h {
        let colr = mode.surf(sh(c::STEEL, 1.3));
        fb.put(spx, spy - dy, colr);
        if s >= 2 && dy < spire_h / 2 {
            fb.put(spx + 1, spy - dy, colr);
        }
    }

    // Aviation beacon at the top of the spire
    let (bx, by) = (spx, spy - spire_h);
    fb.put(bx, by, col::BEACON);
    if s >= 2 {
        fb.put(bx, by - 1, col::BEACON);
    }
    if mode == Mode::Night {
        parts::glow(fb, bx, by, 4 * s, 3 * s, 0.7, col::BEACON);
        // Architectural crown edge lighting
        let (ax, ay) = g.pi_f(ux + 0.3, y + d - 0.7, c1);
        let (bx2, by2) = g.pi_f(x + w - 0.7, y + d - 0.7, c1);
        fb.line((ax as f32, ay as f32), (bx2 as f32, by2 as f32), rgb(0x66, 0xd8, 0xff));
        let (cx, cy) = g.pi_f(x + w - 0.7, uy + 0.3, c1);
        fb.line((bx2 as f32, by2 as f32), (cx as f32, cy as f32), rgb(0x44, 0xa8, 0xd8));
        // Terrace edge lighting
        let (tax, tay) = g.pi_f(x, y + d, pod_h + shaft_h);
        let (tbx, tby) = g.pi_f(x + w, y + d, pod_h + shaft_h);
        fb.line((tax as f32, tay as f32), (tbx as f32, tby as f32), rgb(0x55, 0xb8, 0xf0));
    }
}

/// Tower podium: stone/retail band.
fn podium(face: &Quad, fb: &mut Framebuffer, mode: Mode, side: f32, s: i32, seed: u64) {
    face.paint(fb, &mut |u, v, _x, y| {
        let joint = u % (6 * s) == 0 || v % (4 * s) == 0;
        let base = sh(c::CONCRETE, 0.92);
        if joint {
            mode.surf(sh(base, 0.8 * side))
        } else {
            mode.surf(sh(base, side * (0.94 + 0.08 * col::hash01(u, y))))
        }
    });
    // ground glass band
    let rows = face.rows();
    let gq = face.sub_px(0, rows - 4 * s, face.top_len(), rows, rows as f32);
    let lg = match mode {
        Mode::Day => sh(c::GLASS_B, 0.9),
        Mode::Night => rgb(0xff, 0xd8, 0x96),
    };
    gq.paint(fb, &mut |u, v, _, _| {
        if u % (6 * s) == 0 || v < s / 2 + 1 {
            mode.surf(c::MULLION)
        } else {
            sh(lg, side * 0.95)
        }
    });
    let _ = seed;
}

// ---------------------------------------------------------------------------
// Fillers (background density at the same texturing rules)
// ---------------------------------------------------------------------------

/// A small varied house (1-2 floors, clap/stucco/brick, gable/flat).
pub fn filler_house(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let mut rng = map::rng::Rng::new(seed);
    let w = 1.6 + rng.f32() * 1.2;
    let d = 1.6 + rng.f32() * 1.0;
    let floors = 1 + rng.usize(2) as i32;
    let z1 = floors * RES_FLOOR * s - 2 * s;
    let wall_pick = rng.usize(4);
    let (wc, tex): (Color, u8) = match wall_pick {
        0 => (c::CLAP, 0),
        1 => (c::BRICK, 1),
        2 => (c::STUCCO, 2),
        _ => (sh(c::CLAP, 0.85 + rng.f32() * 0.3), 0),
    };
    let hue = rng.f32() * 0.2 - 0.1;
    let wall = sh(wc, 1.0 + hue);

    parts::building_shadow(fb, g, x, y, x + w, y + d, z1, mode);
    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    match tex {
        0 => {
            parts::paint_clap(&lf, fb, mode, wall, parts::FACE_L, s);
            parts::paint_clap(&rf, fb, mode, sh(wall, 0.9), parts::FACE_R, s);
        }
        1 => {
            parts::paint_brick(&lf, fb, mode, wall, parts::FACE_L, s);
            parts::paint_brick(&rf, fb, mode, sh(wall, 0.9), parts::FACE_R, s);
        }
        _ => {
            parts::paint_stucco(&lf, fb, mode, wall, parts::FACE_L, s);
            parts::paint_stucco(&rf, fb, mode, sh(wall, 0.9), parts::FACE_R, s);
        }
    }
    // windows
    let wo = WinOpts {
        w: (2 * s + 1).max(2),
        h: 3 * s,
        frame: c::TRIM,
        sill: s >= 2,
        shutters: false,
        shutter_c: c::TRIM,
        seed,
        lit_p: 0.6,
    };
    let cols_l = (lf.top_len() / (6 * s)).max(1);
    let cols_r = (rf.top_len() / (6 * s)).max(1);
    parts::window_grid(&lf, fb, mode, cols_l, floors, 2 * s, s, RES_FLOOR * s, wo, s);
    let mut wo2 = wo;
    wo2.seed = seed + 31;
    parts::window_grid(&rf, fb, mode, cols_r, floors, 2 * s, s, RES_FLOOR * s, wo2, s);

    if rng.chance(0.7) {
        let rise = (4 + rng.usize(3) as i32) * s;
        parts::roof_gable(fb, g, mode, x - 0.12, y - 0.12, x + w + 0.12, y + d + 0.12, z1, rise,
            if rng.chance(0.5) { c::ROOF_SHINGLE } else { rgb(0x76, 0x5a, 0x48) });
    } else {
        parts::roof_flat(fb, g, mode, x, y, x + w, y + d, z1, sh(c::CONCRETE, 0.9), c::PARAPET, false, seed);
    }
}

/// A small shop/office (2-4 floors, flat roof, storefront band).
pub fn filler_shop(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let mut rng = map::rng::Rng::new(seed);
    let w = 1.8 + rng.f32() * 1.4;
    let d = 1.8 + rng.f32() * 1.2;
    let floors = 2 + rng.usize(3) as i32;
    let z1 = floors * SHOP_FLOOR * s;
    let tint = 0.8 + rng.f32() * 0.45;
    let base = if rng.chance(0.6) { sh(c::BRICK, tint) } else { sh(c::STUCCO, tint) };

    parts::building_shadow(fb, g, x, y, x + w, y + d, z1, mode);
    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    let rows = lf.rows();
    let lf_up = lf.sub_px(0, 0, lf.top_len(), rows - SHOP_GROUND * s, rows as f32);
    let rf_up = rf.sub_px(0, 0, rf.top_len(), rows - SHOP_GROUND * s, rows as f32);
    parts::paint_brick(&lf_up, fb, mode, base, parts::FACE_L, s);
    parts::paint_brick(&rf_up, fb, mode, sh(base, 0.9), parts::FACE_R, s);
    // storefront glass band
    let gq = lf.sub_px(0, rows - SHOP_GROUND * s + 2 * s, lf.top_len(), rows, rows as f32);
    let lg = match mode {
        Mode::Day => sh(c::GLASS_B, 0.85),
        Mode::Night => {
            if rng.chance(0.6) { rgb(0xff, 0xd4, 0x8c) } else { sh(col::WIN_OFF, 0.7) }
        }
    };
    gq.paint(fb, &mut |u, v, _, _| {
        if u % (5 * s) == 0 || v == 0 {
            mode.surf(c::MULLION)
        } else {
            sh(lg, 0.95)
        }
    });
    // upper windows
    let wo = WinOpts {
        w: (2 * s + 1).max(2),
        h: 3 * s,
        frame: sh(base, 0.6),
        sill: false,
        shutters: false,
        shutter_c: c::TRIM,
        seed,
        lit_p: 0.5,
    };
    let cols_l = (lf.top_len() / (5 * s)).max(1);
    parts::window_grid(&lf_up, fb, mode, cols_l, floors - 1, 2 * s, s, SHOP_FLOOR * s, wo, s);
    let mut wo2 = wo;
    wo2.seed = seed + 17;
    parts::window_grid(&rf_up, fb, mode, (rf.top_len() / (5 * s)).max(1), floors - 1, 2 * s, s, SHOP_FLOOR * s, wo2, s);
    parts::roof_flat(fb, g, mode, x, y, x + w, y + d, z1, sh(c::CONCRETE, 0.9), sh(base, 0.7), s >= 2 && rng.chance(0.5), seed);
}

/// A mid-rise concrete/glass box (6-12 floors).
pub fn filler_mid(fb: &mut Framebuffer, g: &Grid, mode: Mode, x: f32, y: f32, seed: u64) {
    let s = g.s;
    let mut rng = map::rng::Rng::new(seed);
    let w = 2.0 + rng.f32() * 1.2;
    let d = 2.0 + rng.f32() * 1.2;
    let floors = 6 + rng.usize(7) as i32;
    let fl = TOWER_FLOOR * s;
    let z1 = floors * fl;
    let base = sh(c::CONCRETE, 0.85 + rng.f32() * 0.3);

    parts::building_shadow(fb, g, x, y, x + w, y + d, z1, mode);
    let (lf, rf) = box_faces(g, x, y, x + w, y + d, 0, z1);
    // strip windows with concrete spandrels
    strip_windows(&lf, fb, mode, base, parts::FACE_L, s, seed, fl);
    strip_windows(&rf, fb, mode, sh(base, 0.92), parts::FACE_R, s, seed + 1, fl);
    parts::roof_flat(fb, g, mode, x, y, x + w, y + d, z1, sh(c::CONCRETE, 0.8), c::PARAPET, s >= 2, seed);
}

fn strip_windows(face: &Quad, fb: &mut Framebuffer, mode: Mode, base: Color, side: f32, s: i32, seed: u64, fl: i32) {
    face.paint(fb, &mut |u, v, _x, y| {
        let in_band = (v % fl) >= (fl / 3) && (v % fl) <= fl - fl / 4;
        let mull = in_band && u % (3 * s) == 0;
        let jit = 0.92 + 0.1 * col::hash01(u, y);
        if mull {
            mode.surf(sh(c::MULLION, side))
        } else if in_band {
            match mode {
                Mode::Day => mode.surf(sh(c::GLASS_B, side * 0.95)),
                Mode::Night => {
                    let h = col_hash(seed, (u * 3 + v) as u64);
                    if h % 100 < 40 {
                        col::WIN_WARM
                    } else {
                        sh(c::GLASS_B, 0.3)
                    }
                }
            }
        } else {
            mode.surf(sh(base, side * jit))
        }
    });
}
