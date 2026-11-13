//! Suburban Houses: Ranch House, Split-Level, Craftsman, Contemporary.

use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_ranch(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, siding_c, roof_c, is_contemporary) = match v {
        0 => (16, 10, pal::CLAP_WHITE, pal::ROOF_ASPHALT_DARK, false), // 1 storey + 2-car garage + brick wainscot
        1 => (16, 10, pal::CLAP_CREAM, pal::ROOF_CEDAR_SHAKE, false), // cedar shingle + single garage + front veranda
        2 => (26, 12, pal::CLAP_SAGE, pal::ROOF_ASPHALT_DARK, false), // 1.5 storeys (+dormer!) + double garage
        _ => (18, 6, pal::STONE_DARK_SLATE, pal::METAL_STEEL_DARK, true), // 1 storey modern flat/butterfly roof
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_clapboard(&lf, sprite, light, siding_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, siding_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::TRIM_WHITE, ox, oy);

    // Brick wainscot along lower 4px
    let len = rf.top_len();
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        if v_win >= z_wall - 5 {
            let is_m = (v_win % 2 == 0) || (u % 4 == 0);
            let c = if is_m { pal::BRICK_MORTAR } else { pal::BRICK_RED };
            Some((color::surf(c, light, FACE_R), (z_wall - v_win) as u16))
        } else {
            None
        }
    });

    if is_contemporary {
        parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 3, pal::METAL_STEEL_DARK, ox, oy, age);
    } else {
        parts::roof_hip_tile(sprite, light, -0.10, -0.10, w + 0.10, d + 0.10, z_wall, rise, roof_c, ox, oy, age);
    }

    // Attached garage on right face
    let gq = rf.sub_px(4, z_wall - 14, 22, z_wall, rf.rows() as f32);
    gq.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_border = u == 0 || u == 17 || v_win == 0;
        let is_panel = (v_win % 3) == 0 || (u % 4) == 0;
        let c = if is_border || is_panel { pal::METAL_STEEL_DARK } else { pal::TRIM_WHITE };
        Some((color::surf(c, light, FACE_R), (z_wall - v_win) as u16))
    });

    // Wide living room picture window
    let wo = WinOpts {
        w: 12,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c: pal::SHUTTER_GREEN,
        seed: spec.seed,
        lit_p: 0.70,
    };
    parts::draw_window(&rf, sprite, light, len - 16, z_wall - 13, wo, z_wall);
    parts::draw_door(&rf, sprite, light, len - 26, z_wall - 12, 6, 11, pal::CLAP_BARN_RED, pal::TRIM_WHITE, 0);

    // Left face windows
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, len_l - 14, z_wall - 12, wo, z_wall);
}

pub fn draw_split_level(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.30f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_high, z_low) = match v {
        0 => (28, 16),
        1 => (38, 22), // 3-level split (+10px!)
        2 => (28, 16), // blue-grey siding + side balcony
        _ => (32, 18), // modern charcoal split
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_high + 8, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_high, ox, oy);
    let siding_c = match v {
        0 => pal::CLAP_SAGE,
        1 => pal::CLAP_CREAM,
        2 => pal::CLAP_BLUE_GREY,
        _ => pal::STONE_DARK_SLATE,
    };
    parts::paint_clapboard(&lf, sprite, light, siding_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, siding_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_high, pal::TRIM_WHITE, ox, oy);

    // Brick lower storey on left half
    let len = rf.top_len();
    let half_len = len / 2;
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        if u < half_len && v_win >= 14 {
            let is_m = (v_win % 2 == 0) || (u % 4 == 0);
            let c = if is_m { pal::BRICK_MORTAR } else { pal::BRICK_BUFF };
            Some((color::surf(c, light, FACE_R), (z_high - v_win) as u16))
        } else {
            None
        }
    });

    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_high as f32, ox, oy), project(w, d, z_high as f32, ox, oy), project(w, my, (z_high + 8) as f32, ox, oy), siding_c, FACE_R, z_high, z_high + 8);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_high as f32, ox, oy), project(0.0, d, z_high as f32, ox, oy), project(0.0, my, (z_high + 8) as f32, ox, oy), siding_c, FACE_L, z_high, z_high + 8);

    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_high, 8, pal::ROOF_ASPHALT_DARK, ox, oy, age);

    // Integrated garage on left
    let gq = rf.sub_px(3, z_high - 13, 16, z_high, rf.rows() as f32);
    gq.paint(sprite, &mut |u, _v, _x, _y| {
        let is_border = u == 0 || u == 12;
        let c = if is_border { pal::METAL_STEEL_DARK } else { pal::TRIM_WHITE };
        Some((color::surf(c, light, FACE_R), z_low as u16))
    });

    // Projecting bay window on right upper floor
    let wo = WinOpts {
        w: 9,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    parts::draw_window(&rf, sprite, light, half_len + 5, 4, wo, z_high);
    parts::draw_door(&rf, sprite, light, half_len + 18, z_high - 13, 7, 12, pal::SHUTTER_BLACK, pal::TRIM_WHITE, 0);

    // Left face windows
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, len_l - 12, 4, wo, z_high);
}

pub fn draw_craftsman(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, clap_c, roof_c, has_shed_dormer) = match v {
        0 => (28, 12, pal::CLAP_BARN_RED, pal::ROOF_CEDAR_SHAKE, false),
        1 => (20, 14, pal::CLAP_SAGE, pal::ROOF_ASPHALT_DARK, true), // 1.5 storeys + broad shed dormer
        2 => (32, 12, pal::CLAP_PALE_YELLOW, pal::ROOF_CEDAR_SHAKE, false), // 2 storeys + double front gables
        _ => (28, 12, pal::STONE_DARK_SLATE, pal::ROOF_SLATE_GREY, false), // charcoal + stone chimney
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_clapboard(&lf, sprite, light, clap_c, FACE_L, age);
    parts::paint_clapboard(&rf, sprite, light, clap_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::TRIM_WHITE, ox, oy);

    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_R, z_wall, z_wall + rise);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + rise) as f32, ox, oy), clap_c, FACE_L, z_wall, z_wall + rise);

    parts::roof_gable(sprite, light, -0.10, -0.10, w + 0.10, d + 0.10, z_wall, rise, roof_c, ox, oy, age);

    if has_shed_dormer {
        parts::draw_dormer(sprite, light, w * 0.5, d * 0.45, z_wall + 3, 9, ox, oy);
    }

    // Craftsman covered front porch: attached firmly to house wall, solid floor deck, tapered stone piers, posts under roof
    let porch_d = 0.40f32;
    let porch_h = 13;
    // Solid porch deck floor on the ground
    let p_deck = box_top(0.0, d, w, d + porch_d, 2, ox, oy);
    p_deck.fill(sprite, color::surf(pal::STONE_BROWNSTONE, light, FACE_TOP));
    let (pd_lf, pd_rf) = box_faces(0.0, d, w, d + porch_d, 0, 2, ox, oy);
    pd_lf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
    pd_rf.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));

    // Porch roof attached directly to the house wall at y=d
    let p_top = box_top(0.0, d, w, d + porch_d, porch_h, ox, oy);
    p_top.fill(sprite, color::surf(pal::ROOF_ASPHALT_DARK, light, FACE_TOP));
    let (pr_lf, pr_rf) = box_faces(0.0, d, w, d + porch_d, porch_h - 2, porch_h, ox, oy);
    pr_lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
    pr_rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));

    // 3 tapered stone piers standing firmly ON THE PORCH DECK from z=2 to z=7
    // with wooden posts continuing from z=7 up to the porch roof at z=porch_h
    for i in 0..3 {
        let px = 0.05 + i as f32 * (w - 0.35) / 2.0;
        let py = d + porch_d - 0.22;
        let (lf_p, rf_p) = box_faces(px, py, px + 0.24, py + 0.20, 2, 7, ox, oy);
        lf_p.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_L));
        rf_p.fill(sprite, color::surf(pal::STONE_GRANITE, light, FACE_R));
        let p_top_slab = box_top(px, py, px + 0.24, py + 0.20, 7, ox, oy);
        p_top_slab.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_TOP));

        // Tapered wooden column post connecting pier top (z=7) to porch roof (z=porch_h)
        let cx = px + 0.12;
        let cy = py + 0.10;
        let (p0_x, p0_y) = project(cx, cy, 7.0, ox, oy);
        let (p1_x, p1_y) = project(cx, cy, porch_h as f32, ox, oy);
        geom::draw_line(sprite, p0_x.round() as i32, p0_y.round() as i32, p1_x.round() as i32, p1_y.round() as i32, color::surf(pal::TRIM_WHITE, light, 1.0), porch_h as u16);
    }

    // Windows
    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    parts::draw_window(&rf, sprite, light, 6, 3, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 12, 3, wo, z_wall);
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 7, 12, pal::STONE_BROWNSTONE, pal::TRIM_WHITE, 0);

    // Left face windows
    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 3, wo, z_wall);
    parts::draw_window(&lf, sprite, light, len_l - 12, 3, wo, z_wall);
}

pub fn draw_contemporary(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.00f32, 1.80f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 30,
        1 => 34, // flat roof + roof garden terrace
        2 => 44, // 3 storeys (+1 storey!) cubic architecture
        _ => 30, // butterfly roof
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let len = rf.top_len();
    let half = len / 2;

    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_cedar = u >= half;
        let c = if is_cedar {
            let lap = (v_win % 3) == 0;
            if lap { color::shade(pal::STONE_BROWNSTONE, 0.85) } else { pal::STONE_BROWNSTONE }
        } else {
            let seam = (u % 4) == 0;
            if seam { pal::METAL_STEEL_DARK } else { color::shade(pal::METAL_STEEL_DARK, 1.2) }
        };
        Some((color::surf(c, light, FACE_R), (z_wall - v_win) as u16))
    });

    let len_l = lf.top_len();
    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_cedar = u >= len_l / 2;
        let c = if is_cedar { pal::STONE_BROWNSTONE } else { pal::METAL_STEEL_DARK };
        Some((color::surf(c, light, FACE_L), (z_wall - v_win) as u16))
    });
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 3, pal::METAL_STEEL_DARK, ox, oy, age);

    // Floor-to-ceiling glass windows
    let wo = WinOpts {
        w: 10,
        h: 12,
        frame: pal::METAL_STEEL_DARK,
        sill: false,
        lintel: false,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.75,
    };
    parts::draw_window(&rf, sprite, light, 5, 3, wo, z_wall);
    parts::draw_window(&rf, sprite, light, half + 5, 3, wo, z_wall);

    // Frosted glass garage door
    let gq = rf.sub_px(4, z_wall - 14, half - 3, z_wall, rf.rows() as f32);
    gq.paint(sprite, &mut |u, _v, _x, _y| {
        let is_frame = u == 0 || u == half - 8;
        let c = if is_frame { pal::METAL_STEEL_DARK } else { pal::GLASS_REFLECTIVE };
        Some((color::surf(c, light, FACE_R), z_wall as u16))
    });
}
