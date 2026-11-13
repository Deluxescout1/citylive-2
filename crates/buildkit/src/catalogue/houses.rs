//! New England Houses: Cape Cod, Colonial, Saltbox, Farmhouse with Porch.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_cape(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, clap_c, shutter_c, roof_c, has_dormers, has_porch) = match v {
        0 => (18, 16, pal::CLAP_WHITE, pal::SHUTTER_GREEN, pal::ROOF_SLATE_GREY, false, false),
        1 => (30, 16, pal::CLAP_BLUE_GREY, pal::SHUTTER_BLACK, pal::ROOF_CEDAR_SHAKE, true, false), // 2 storeys + dormers
        2 => (18, 18, pal::ROOF_CEDAR_SHAKE, pal::CLAP_BARN_RED, pal::ROOF_ASPHALT_DARK, true, false), // cedar shingles + shed dormer
        _ => (16, 15, pal::CLAP_PALE_YELLOW, pal::SHUTTER_BURGUNDY, pal::ROOF_SLATE_GREY, false, true), // 1 storey cottage with front porch
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_clapboard(&lf, sprite, light, clap_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, clap_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::TRIM_WHITE, ox, oy);

    // Gable triangular ends on both ends (left and right faces)
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_L, z_wall, z_wall + rise);
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_R, z_wall, z_wall + rise);

    // Gable roof
    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, roof_c, ox, oy, age);

    // Dormer windows on roof for v1 and v2
    if has_dormers {
        parts::draw_dormer(sprite, light, w * 0.25, d * 0.45, z_wall + 4, 10, ox, oy);
        parts::draw_dormer(sprite, light, w * 0.65, d * 0.45, z_wall + 4, 10, ox, oy);
    }

    // Front porch for v3
    if has_porch {
        let p_top = box_top(0.0, d, w, d + 0.35, 10, ox, oy);
        p_top.fill(sprite, color::surf(pal::ROOF_SLATE_GREY, light, FACE_TOP));
        for i in 0..3 {
            let px = i as f32 * w / 2.0;
            let (p0_x, p0_y) = project(px, d + 0.35, 0.0, ox, oy);
            let (p1_x, p1_y) = project(px, d + 0.35, 10.0, ox, oy);
            geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, color::surf(pal::TRIM_WHITE, light, 1.0), 10);
        }
    }

    // Central brick or stone chimney (penetrates deep into roof attic to eliminate gaps)
    let chimney_c = if v == 3 { pal::STONE_GRANITE } else { pal::BRICK_RED };
    let (cx0, cy0, cw, cd) = (w / 2.0 - 0.10, d / 2.0 - 0.10, 0.20, 0.20);
    let (c_lf, c_rf) = box_faces(cx0, cy0, cx0 + cw, cy0 + cd, z_wall - 2, z_wall + rise + 9, ox, oy);
    c_lf.fill(sprite, color::surf(chimney_c, light, FACE_L));
    c_rf.fill(sprite, color::surf(color::shade(chimney_c, 0.8), light, FACE_R));
    let c_top = box_top(cx0, cy0, cx0 + cw, cy0 + cd, z_wall + rise + 9, ox, oy);
    c_top.fill(sprite, color::surf(pal::BRICK_MORTAR, light, FACE_TOP));

    // Windows on right face
    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c,
        seed: spec.seed,
        lit_p: 0.70,
    };
    parts::draw_window(&rf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 12, 4, wo, z_wall);

    // Windows on left face
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&lf, sprite, light, len_l - 12, 4, wo, z_wall);

    // Front door centered on right face
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 7, 12, pal::CLAP_BARN_RED, pal::TRIM_WHITE, 0);
}

pub fn draw_colonial(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.10f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, clap_c, shutter_c, roof_c, is_brick, has_portico) = match v {
        0 => (32, 15, pal::CLAP_PALE_YELLOW, pal::SHUTTER_GREEN, pal::ROOF_ASPHALT_DARK, false, false),
        1 => (44, 15, pal::CLAP_SAGE, pal::SHUTTER_BURGUNDY, pal::ROOF_SLATE_GREY, false, true), // 3 storeys + grand portico
        2 => (32, 16, pal::CLAP_WHITE, pal::SHUTTER_BLACK, pal::ROOF_CEDAR_SHAKE, false, false), // 2 storeys + cupola
        _ => (38, 16, pal::BRICK_RED, pal::SHUTTER_BLACK, pal::ROOF_SLATE_GREY, true, true), // 2.5 storeys red brick Georgian
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    if is_brick {
        parts::paint_brick(&lf, sprite, light, clap_c, pal::BRICK_MORTAR, FACE_L, age);
        parts::paint_brick(&rf, sprite, light, clap_c, pal::BRICK_MORTAR, FACE_R, age);
    } else {
        parts::paint_clapboard(&lf, sprite, light, clap_c, FACE_L, age);
        parts::paint_clapboard(&rf, sprite, light, clap_c, FACE_R, age);
    }
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::TRIM_WHITE, ox, oy);

    // Gable ends
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_R, z_wall, z_wall + rise);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_L, z_wall, z_wall + rise);

    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, roof_c, ox, oy, age);

    // Cupola on roof for v2
    if v == 2 {
        let (cx, cy) = (w / 2.0 - 0.15, d / 2.0 - 0.15);
        let (c_lf, c_rf) = box_faces(cx, cy, cx + 0.30, cy + 0.30, z_wall + rise - 4, z_wall + rise + 10, ox, oy);
        c_lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
        c_rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));
        parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_wall + rise + 10, 6, ox, oy);
    }

    // Twin chimneys
    for &cx in &[0.20f32, w - 0.40f32] {
        let (cy, cw, cd) = (d / 2.0 - 0.10, 0.20, 0.20);
        let (c_lf, c_rf) = box_faces(cx, cy, cx + cw, cy + cd, z_wall - 2, z_wall + rise + 10, ox, oy);
        c_lf.fill(sprite, color::surf(pal::BRICK_RED, light, FACE_L));
        c_rf.fill(sprite, color::surf(color::shade(pal::BRICK_RED, 0.8), light, FACE_R));
        let c_top = box_top(cx, cy, cx + cw, cy + cd, z_wall + rise + 10, ox, oy);
        c_top.fill(sprite, color::surf(pal::BRICK_MORTAR, light, FACE_TOP));
    }

    // Symmetrical windows on right face
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let len = rf.top_len();
    let num_floors = if z_wall > 40 { 3 } else { 2 };
    for floor in 0..num_floors {
        let v_win = 3 + floor * 12;
        for col in 0..5 {
            let u = 4 + col * (len - 8) / 4;
            if !(floor == num_floors - 1 && col == 2) { // Skip center ground floor for door
                parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 12);
            }
        }
    }

    // Left face windows
    let len_l = lf.top_len();
    for floor in 0..num_floors {
        let v_win = 3 + floor * 12;
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 12);
    }

    // Grand entrance portico or doorway
    if has_portico {
        let p_top = box_top(w / 2.0 - 0.35, d, w / 2.0 + 0.35, d + 0.30, z_wall - 16, ox, oy);
        p_top.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_TOP));
        let (p0_x, p0_y) = project(w / 2.0 - 0.30, d + 0.28, 0.0, ox, oy);
        let (p1_x, p1_y) = project(w / 2.0 - 0.30, d + 0.28, (z_wall - 16) as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, color::surf(pal::TRIM_WHITE, light, 1.0), (z_wall - 16) as u16);
    }
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 8, 12, pal::SHUTTER_BLACK, pal::TRIM_WHITE, 0);
}

pub fn draw_saltbox(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.25f32, 1.10f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_front, z_rear, rise, clap_c, roof_c, has_dormer, has_porch) = match v {
        0 => (28, 14, 12, pal::CLAP_CREAM, pal::ROOF_CEDAR_SHAKE, false, false),
        1 => (28, 12, 14, pal::ROOF_CEDAR_SHAKE, pal::ROOF_ASPHALT_DARK, false, true), // dark weathered cedar + entry porch
        2 => (32, 16, 12, pal::CLAP_BARN_RED, pal::ROOF_SLATE_GREY, true, false), // barn red + roof dormer
        _ => (26, 12, 12, pal::CLAP_WHITE, pal::ROOF_CEDAR_SHAKE, false, false), // white saltbox with side ell
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_front + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_front, ox, oy);
    parts::paint_clapboard(&lf, sprite, light, clap_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, clap_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_front, pal::TRIM_WHITE, ox, oy);

    // Asymmetrical saltbox gable ends on left and right faces
    let ridge_y = d * 0.35;
    let tri_saltbox_r = [
        project(w, 0.0, z_rear as f32, ox, oy),
        project(w, ridge_y, (z_front + rise) as f32, ox, oy),
        project(w, d, z_front as f32, ox, oy),
        project(w, 0.0, z_front as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &tri_saltbox_r, color::surf(clap_c, light, FACE_R), (z_front + rise) as u16);
    let tri_saltbox_l = [
        project(0.0, 0.0, z_rear as f32, ox, oy),
        project(0.0, ridge_y, (z_front + rise) as f32, ox, oy),
        project(0.0, d, z_front as f32, ox, oy),
        project(0.0, 0.0, z_front as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &tri_saltbox_l, color::surf(clap_c, light, FACE_L), (z_front + rise) as u16);

    // Asymmetrical saltbox roof profile (long lean-to slope down the back)
    let slope_front = [
        project(-0.05, ridge_y, (z_front + rise) as f32, ox, oy),
        project(w + 0.05, ridge_y, (z_front + rise) as f32, ox, oy),
        project(w + 0.05, d + 0.05, z_front as f32, ox, oy),
        project(-0.05, d + 0.05, z_front as f32, ox, oy),
    ];
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    geom::fill_polygon(sprite, &slope_front, color::surf(roof_c, light, 0.92), (z_front + rise) as u16);

    let slope_back = [
        project(-0.05, -0.05, z_rear as f32, ox, oy),
        project(w + 0.05, -0.05, z_rear as f32, ox, oy),
        project(w + 0.05, ridge_y, (z_front + rise) as f32, ox, oy),
        project(-0.05, ridge_y, (z_front + rise) as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &slope_back, color::surf(roof_c, light, FACE_TOP), (z_front + rise) as u16);
    sprite.on_surface(render::snow::NONE);

    // Roof dormer on front slope for v2
    if has_dormer {
        parts::draw_dormer(sprite, light, w * 0.5, d * 0.55, z_front + 2, 8, ox, oy);
    }

    // Front porch overhang for v1
    if has_porch {
        let p = box_top(w / 2.0 - 0.30, d, w / 2.0 + 0.30, d + 0.25, 10, ox, oy);
        p.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_TOP));
    }

    // Massive central chimney
    let (cx, cy, cw, cd) = (w / 2.0 - 0.12, ridge_y - 0.08, 0.24, 0.20);
    let (c_lf, c_rf) = box_faces(cx, cy, cx + cw, cy + cd, z_front - 2, z_front + rise + 10, ox, oy);
    c_lf.fill(sprite, color::surf(pal::BRICK_DARK_CLINKER, light, FACE_L));
    c_rf.fill(sprite, color::surf(color::shade(pal::BRICK_DARK_CLINKER, 0.8), light, FACE_R));

    // Windows on right face
    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let len = rf.top_len();
    parts::draw_window(&rf, sprite, light, 6, 4, wo, z_front);
    parts::draw_window(&rf, sprite, light, len - 12, 4, wo, z_front);
    parts::draw_window(&rf, sprite, light, 6, 16, wo, z_front);
    parts::draw_door(&rf, sprite, light, len / 2 - 4, 16, 7, 11, pal::CLAP_BARN_RED, pal::TRIM_WHITE, 0);

    // Windows on left face
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 4, wo, z_front);
    parts::draw_window(&lf, sprite, light, len_l - 12, 4, wo, z_front);
}

pub fn draw_farmhouse(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, clap_c, shutter_c, roof_c, is_double_porch) = match v {
        0 => (28, 16, pal::CLAP_WHITE, pal::SHUTTER_GREEN, pal::ROOF_SLATE_GREY, false),
        1 => (36, 16, pal::CLAP_BARN_RED, pal::TRIM_WHITE, roof_tin_red(), false), // 2.5 storeys + attic gable + cupola
        2 => (32, 16, pal::CLAP_PALE_YELLOW, pal::SHUTTER_BLACK, pal::ROOF_ASPHALT_DARK, true), // 2 storeys + 2-tier double porch!
        _ => (22, 14, pal::CLAP_SAGE, pal::SHUTTER_BURGUNDY, pal::ROOF_CEDAR_SHAKE, false), // 1.5 storeys + screened veranda
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_clapboard(&lf, sprite, light, clap_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, clap_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::TRIM_WHITE, ox, oy);

    // Gable triangular ends
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_R, z_wall, z_wall + rise);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_L, z_wall, z_wall + rise);

    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, roof_c, ox, oy, age);

    // Rooftop cupola for v1
    if v == 1 {
        let (cx, cy) = (w / 2.0 - 0.15, d / 2.0 - 0.15);
        let (c_lf, c_rf) = box_faces(cx, cy, cx + 0.30, cy + 0.30, z_wall + rise - 4, z_wall + rise + 8, ox, oy);
        c_lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
        c_rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));
        parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_wall + rise + 8, 6, ox, oy);
    }

    // Porch construction (attached firmly to wall, solid deck floor, posts on deck)
    let porch_d = 0.40f32;
    let porch_h = 12;

    // Solid porch deck floor on the ground
    let p_deck = box_top(0.0, d, w, d + porch_d, 2, ox, oy);
    p_deck.fill(sprite, color::surf(pal::STONE_BROWNSTONE, light, FACE_TOP));
    let (pd_lf, pd_rf) = box_faces(0.0, d, w, d + porch_d, 0, 2, ox, oy);
    pd_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    pd_rf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));

    // Porch roof attached directly to house wall at y=d
    let p_top = box_top(0.0, d, w, d + porch_d, porch_h, ox, oy);
    p_top.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));
    let (pr_lf, pr_rf) = box_faces(0.0, d, w, d + porch_d, porch_h - 2, porch_h, ox, oy);
    pr_lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
    pr_rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));

    // Turned white porch posts standing firmly ON the porch deck (from z=2 to z=porch_h)
    let post_c = color::surf(pal::TRIM_WHITE, light, 1.0);
    for i in 0..4 {
        let px = 0.05 + i as f32 * (w - 0.10) / 3.0;
        let py = d + porch_d - 0.05;
        let (p0_x, p0_y) = project(px, py, 2.0, ox, oy);
        let (p1_x, p1_y) = project(px, py, porch_h as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, post_c, porch_h as u16);
    }
    // Handrail balustrade at z=5
    let (r0_x, r0_y) = project(0.05, d + porch_d - 0.05, 5.0, ox, oy);
    let (r1_x, r1_y) = project(w - 0.05, d + porch_d - 0.05, 5.0, ox, oy);
    geom::draw_line(sprite, r0_x.round() as i32, r0_y.round() as i32, r1_x.round() as i32, r1_y.round() as i32, post_c, 5);

    // Double-decker upper porch for v2
    if is_double_porch {
        let p2_top = box_top(0.0, d, w, d + porch_d, porch_h + 12, ox, oy);
        p2_top.fill(sprite, color::surf(pal::ROOF_GRAVEL, light, FACE_TOP));
        for i in 0..4 {
            let px = 0.05 + i as f32 * (w - 0.10) / 3.0;
            let py = d + porch_d - 0.05;
            let (p0_x, p0_y) = project(px, py, porch_h as f32, ox, oy);
            let (p1_x, p1_y) = project(px, py, (porch_h + 12) as f32, ox, oy);
            geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, post_c, (porch_h + 12) as u16);
        }
    }

    // Windows
    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c,
        seed: spec.seed,
        lit_p: 0.70,
    };
    parts::draw_window(&rf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 12, 4, wo, z_wall);
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 7, 11, pal::TRIM_WHITE, pal::TRIM_WHITE, 0);

    // Left face windows
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&lf, sprite, light, len_l - 12, 4, wo, z_wall);
}

fn roof_tin_red() -> render::palette::Color {
    rgb(0x8a, 0x34, 0x2e)
}
