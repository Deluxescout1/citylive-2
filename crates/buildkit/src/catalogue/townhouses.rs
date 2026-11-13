//! European Townhouses: Haussmann, Mediterranean Stucco, Canal House, Old Town Half-Timbered.

use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_haussmann(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, mansard_h, stone_c) = match v {
        0 => (48, 16, pal::STONE_LIMESTONE),
        1 => (60, 16, pal::STONE_GRANITE), // 5 storeys (+1 storey!)
        2 => (48, 14, pal::STONE_LIMESTONE), // 4 storeys sandstone + arched carriage entrance
        _ => (36, 14, pal::STUCCO_SAND), // 3 storeys + piano nobile
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + mansard_h, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    // French mansard roof with slate and iron cresting
    parts::roof_mansard(sprite, light, 0.0, 0.0, w, d, z_wall, mansard_h, pal::ROOF_SLATE_GREY, ox, oy, age);

    // Wrought-iron balconies on floors with flower boxes
    let len = rf.top_len();
    let num_floors = if z_wall > 50 { 5 } else if z_wall > 40 { 4 } else { 3 };
    for floor in 1..num_floors {
        let v_win = 2 + floor * 11;
        if floor == 1 || floor == num_floors - 2 {
            parts::draw_balcony(&rf, sprite, light, 3, v_win + 5, len - 6, 6, z_wall - v_win);
        }
    }

    // French sash windows on both faces
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.68,
    };
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 11;
        parts::draw_window(&rf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&rf, sprite, light, len - 11, v_win, wo, z_wall - floor * 11);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 11, v_win, wo, z_wall - floor * 11);
    }

    // Ground floor carriage entrance archway
    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 12, 10, 12, pal::SHUTTER_BLACK, pal::STONE_DARK_SLATE, 0);
}

pub fn draw_mediterranean_stucco(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, stucco_c, shutter_c, has_loggia) = match v {
        0 => (38, 14, pal::STUCCO_TERRACOTTA, pal::SHUTTER_GREEN, false),
        1 => (50, 14, pal::STUCCO_OCHRE, pal::SHUTTER_BURGUNDY, true), // 4 storeys + arched loggia
        2 => (38, 14, pal::STUCCO_ROSE, pal::SHUTTER_BLACK, false), // 3 storeys rose stucco + carriage portal
        _ => (28, 12, pal::STUCCO_SAND, pal::SHUTTER_GREEN, false), // 2.5 storeys + tile entry awning
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stucco(&lf, sprite, light, stucco_c, FACE_L, age);
    parts::paint_stucco(&rf, sprite, light, stucco_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    // Terracotta barrel-tile hip roof
    parts::roof_hip_tile(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, pal::ROOF_TERRACOTTA, ox, oy, age);

    // Juliet balconies with blooming red geraniums
    let len = rf.top_len();
    parts::draw_balcony(&rf, sprite, light, 4, 7, 8, 5, z_wall - 7);
    parts::draw_balcony(&rf, sprite, light, len - 12, 7, 8, 5, z_wall - 7);

    // Arched windows with wooden louvered shutters
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_CREAM,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c,
        seed: spec.seed,
        lit_p: 0.72,
    };
    let num_floors = if z_wall > 45 { 4 } else if z_wall > 35 { 3 } else { 2 };
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 12;
        parts::draw_window(&rf, sprite, light, 5, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&rf, sprite, light, len - 11, v_win, wo, z_wall - floor * 12);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 5, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 11, v_win, wo, z_wall - floor * 12);
    }

    if has_loggia {
        // Open arched loggia on top floor
        let lq = rf.sub_px(4, 2, len - 4, 12, rf.rows() as f32);
        lq.fill(sprite, color::surf(pal::STUCCO_SAND, light, 0.85));
    }

    // Arched rustic stone doorway
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 13, 8, 12, pal::STONE_BROWNSTONE, pal::STONE_LIMESTONE, 0);
}

pub fn draw_canal_house(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, gable_rise, brick_c, trim_c) = match v {
        0 => (48, 16, pal::BRICK_DARK_CLINKER, pal::TRIM_WHITE), // 4 storeys + stepped gable
        1 => (60, 18, pal::STONE_DARK_SLATE, pal::STONE_LIMESTONE), // 5 storeys + neck gable with vase finials
        2 => (48, 16, pal::BRICK_RED, pal::TRIM_CREAM), // 4 storeys + bell gable
        _ => (36, 14, pal::STONE_BROWNSTONE, pal::TRIM_WHITE), // 3 storeys + spout gable
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + gable_rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, trim_c, ox, oy);

    // Dutch Stepped/Bell Gable on the front face (rf along x=w)
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + gable_rise) as f32, ox, oy), brick_c, FACE_R, z_wall, z_wall + gable_rise);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + gable_rise) as f32, ox, oy), brick_c, FACE_L, z_wall, z_wall + gable_rise);

    // Pitched roof sloping back from the gable to x=0
    let roof_slope_l = [
        project(0.0, 0.0, z_wall as f32, ox, oy),
        project(w, 0.0, z_wall as f32, ox, oy),
        project(w, my, (z_wall + gable_rise) as f32, ox, oy),
        project(0.0, my, (z_wall + gable_rise) as f32, ox, oy),
    ];
    sprite.on_surface(render::snow::SLOPE_MEDIUM);
    geom::fill_polygon(sprite, &roof_slope_l, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_TOP), (z_wall + gable_rise) as u16);

    let roof_slope_r = [
        project(0.0, my, (z_wall + gable_rise) as f32, ox, oy),
        project(w, my, (z_wall + gable_rise) as f32, ox, oy),
        project(w, d, z_wall as f32, ox, oy),
        project(0.0, d, z_wall as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &roof_slope_r, color::surf(pal::ROOF_CEDAR_SHAKE, light, 0.90), (z_wall + gable_rise) as u16);
    sprite.on_surface(render::snow::NONE);

    // Triangular gable wall on rear left face (x=0)
    let tri_l = [
        project(0.0, 0.0, z_wall as f32, ox, oy),
        project(0.0, d, z_wall as f32, ox, oy),
        project(0.0, my, (z_wall + gable_rise) as f32, ox, oy),
    ];
    geom::fill_polygon(sprite, &tri_l, color::surf(brick_c, light, FACE_L), (z_wall + gable_rise) as u16);

    // Projecting hoist beam at gable apex
    let (apex_x, apex_y) = project(w, my, (z_wall + gable_rise - 2) as f32, ox, oy);
    let beam_c = color::surf(pal::STONE_BROWNSTONE, light, 1.0);
    geom::draw_line(sprite, apex_x.round() as i32, apex_y.round() as i32, apex_x.round() as i32 + 4, apex_y.round() as i32 + 2, beam_c, (z_wall + gable_rise) as u16);

    // Tall narrow Dutch sash windows with transoms on both faces
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: trim_c,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let len = rf.top_len();
    let num_upper = if z_wall > 50 { 4 } else if z_wall > 40 { 3 } else { 2 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 11;
        parts::draw_window(&rf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&rf, sprite, light, len - 11, v_win, wo, z_wall - floor * 11);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 11, v_win, wo, z_wall - floor * 11);
    }

    // Elevated front stoop with iron railings
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 7, 12, pal::SHUTTER_BLACK, trim_c, 0);
}

pub fn draw_half_timbered(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, rise, plaster_c, timber_c, has_dormer) = match v {
        0 => (38, 18, pal::STUCCO_SAND, pal::STONE_BROWNSTONE, false),
        1 => (50, 18, pal::STUCCO_WHITE, pal::SHUTTER_BLACK, true), // 4 storeys (+1 storey!) + dormer
        2 => (38, 18, pal::STUCCO_TERRACOTTA, pal::STONE_DARK_SLATE, false), // 3 storeys terracotta infill
        _ => (28, 16, pal::STUCCO_WHITE, pal::STONE_BROWNSTONE, false), // 2.5 storeys whitewashed
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stucco(&lf, sprite, light, plaster_c, FACE_L, age);
    parts::paint_stucco(&rf, sprite, light, plaster_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, timber_c, ox, oy);

    // Half-timber framework (dark posts, horizontal tie beams, and diagonal cross-braces) on both faces
    rf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_floor_beam = (v_win % 12) == 0;
        // `u` counts half-pixel steps (B99): the face is 2 * top_len() steps wide.
        let is_corner_post = u <= 1 || u >= rf.top_len() * 2 - 3;
        let is_center_post = u == rf.top_len();
        let is_diagonal_brace = ((u + v_win) % 12) == 0 && (v_win > 12);

        if is_floor_beam || is_corner_post || is_center_post || is_diagonal_brace {
            let weathered = color::weather(timber_c, u, v_win, age);
            Some((color::surf(weathered, light, FACE_R), (z_wall - v_win) as u16))
        } else {
            None
        }
    });

    lf.paint(sprite, &mut |u, v_win, _x, _y| {
        let is_floor_beam = (v_win % 12) == 0;
        let is_corner_post = u <= 1 || u >= lf.top_len() * 2 - 3;
        let is_center_post = u == lf.top_len();
        let is_diagonal_brace = ((u + v_win) % 12) == 0 && (v_win > 12);

        if is_floor_beam || is_corner_post || is_center_post || is_diagonal_brace {
            let weathered = color::weather(timber_c, u, v_win, age);
            Some((color::surf(weathered, light, FACE_L), (z_wall - v_win) as u16))
        } else {
            None
        }
    });

    // Triangular gable on front right face and rear left face
    let my = d / 2.0;
    parts::draw_gable_end(sprite, light, project(w, 0.0, z_wall as f32, ox, oy), project(w, d, z_wall as f32, ox, oy), project(w, my, (z_wall + rise) as f32, ox, oy), plaster_c, FACE_R, z_wall, z_wall + rise);
    parts::draw_gable_end(sprite, light, project(0.0, 0.0, z_wall as f32, ox, oy), project(0.0, d, z_wall as f32, ox, oy), project(0.0, my, (z_wall + rise) as f32, ox, oy), plaster_c, FACE_L, z_wall, z_wall + rise);

    // Steep tile gable roof
    parts::roof_gable(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, pal::ROOF_TERRACOTTA, ox, oy, age);

    if has_dormer {
        parts::draw_dormer(sprite, light, w * 0.5, d * 0.45, z_wall + 4, 10, ox, oy);
    }

    // Diamond-pane casement windows
    let len = rf.top_len();
    let wo = WinOpts {
        w: 5,
        h: 7,
        frame: timber_c,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: timber_c,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let num_upper = if z_wall > 45 { 3 } else if z_wall > 35 { 2 } else { 1 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 12;
        parts::draw_window(&rf, sprite, light, 4, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&rf, sprite, light, len - 10, v_win, wo, z_wall - floor * 12);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 4, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 10, v_win, wo, z_wall - floor * 12);
    }

    // Old oak door
    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 7, 12, timber_c, timber_c, 0);
}
