//! Civic Buildings: School, Town Hall, Library, Fire Station, Hospital.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{box_faces, box_top, project};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_school(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.90f32, 1.90f32);
    let v = spec.variant % 4;
    let (z_wall, cupola_h, brick_c) = match v {
        0 => (28, 16, pal::BRICK_RED), // 2 storeys
        1 => (40, 18, pal::BRICK_DARK_CLINKER), // 3 storeys (+1 storey!)
        2 => (28, 14, pal::BRICK_BUFF), // 2 storeys modern addition
        _ => (32, 16, pal::STONE_LIMESTONE), // 2 storeys Collegiate Gothic
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + cupola_h, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);

    // Central cupola with school bell
    let (cx, cy, cw, cd) = (w / 2.0 - 0.22, d / 2.0 - 0.22, 0.44, 0.44);
    let (c_lf, c_rf) = box_faces(cx, cy, cx + cw, cy + cd, z_wall + 4, z_wall + 4 + cupola_h, ox, oy);
    c_lf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_L));
    c_rf.fill(sprite, color::surf(pal::TRIM_WHITE, light, FACE_R));
    parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_wall + 4 + cupola_h, 8, ox, oy);

    // Large classroom windows on both faces
    let len = rf.top_len();
    let wo = WinOpts {
        w: 7,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let num_floors = if z_wall > 35 { 3 } else { 2 };
    for floor in 0..num_floors {
        let v_win = 3 + floor * 12;
        for bay in 0..6 {
            let u = 4 + bay * (len - 8) / 5;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 12);
        }
        let len_l = lf.top_len();
        for bay in 0..4 {
            let u = 4 + bay * (len_l - 8) / 3;
            parts::draw_window(&lf, sprite, light, u, v_win, wo, z_wall - floor * 12);
        }
    }

    // Central portico entrance
    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 13, 10, 13, pal::STONE_BROWNSTONE, pal::TRIM_WHITE, 0);
}

pub fn draw_town_hall(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let (z_wall, tower_h, stone_c) = match v {
        0 => (42, 32, pal::STONE_LIMESTONE), // 3 storeys
        1 => (54, 38, pal::STONE_LIMESTONE), // 4 storeys (+1 storey!) + 6 columns
        2 => (42, 30, pal::BRICK_RED), // red brick + marble quoins
        _ => (46, 32, pal::STONE_GRANITE), // Beaux-Arts palace
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + tower_h, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    if v == 2 {
        parts::paint_brick(&lf, sprite, light, stone_c, pal::BRICK_MORTAR, FACE_L, age);
        parts::paint_brick(&rf, sprite, light, stone_c, pal::BRICK_MORTAR, FACE_R, age);
    } else {
        parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
        parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    }
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 5, pal::STONE_GRANITE, ox, oy, age);

    // Central Clock Tower
    let (tx, ty, tw, td) = (w / 2.0 - 0.32, d / 2.0 - 0.32, 0.64, 0.64);
    let (t_lf, t_rf) = box_faces(tx, ty, tx + tw, ty + td, z_wall + 5, z_wall + 5 + tower_h, ox, oy);
    t_lf.fill(sprite, color::surf(stone_c, light, FACE_L));
    t_rf.fill(sprite, color::surf(color::shade(stone_c, 0.85), light, FACE_R));

    // Illuminated clock on tower
    let (clock_x, clock_y) = project(tx + tw / 2.0, ty + td, (z_wall + 5 + tower_h - 10) as f32, ox, oy);
    let clock_c = if light.is_night { color::WIN_WARM } else { pal::TRIM_WHITE };
    for dy in -3..=3 {
        for dx in -3..=3 {
            if dx * dx + dy * dy <= 8 {
                sprite.set_pixel(clock_x.round() as i32 + dx, clock_y.round() as i32 + dy, clock_c, (z_wall + 5 + tower_h) as u16);
            }
        }
    }
    sprite.set_pixel(clock_x.round() as i32, clock_y.round() as i32, pal::SHUTTER_BLACK, (z_wall + 5 + tower_h) as u16);
    sprite.set_pixel(clock_x.round() as i32, clock_y.round() as i32 - 1, pal::SHUTTER_BLACK, (z_wall + 5 + tower_h) as u16);

    parts::spire_needle(sprite, light, tx + tw / 2.0, ty + td / 2.0, z_wall + 5 + tower_h, 14, ox, oy);

    // Windows on both faces
    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::STONE_LIMESTONE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let num_floors = if z_wall > 48 { 4 } else { 3 };
    for floor in 0..num_floors - 1 {
        let v_win = 3 + floor * 12;
        parts::draw_window(&rf, sprite, light, 6, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&rf, sprite, light, len - 12, v_win, wo, z_wall - floor * 12);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 12);
    }

    // Monumental columned entrance portico
    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 14, 10, 14, pal::METAL_CHAMPAGNE, pal::STONE_GRANITE, 0);
}

pub fn draw_library(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let (z_wall, rise, stone_c, roof_c) = match v {
        0 => (30, 14, pal::STONE_LIMESTONE, pal::ROOF_COPPER_PATINA),
        1 => (42, 16, pal::STONE_LIMESTONE, pal::ROOF_SLATE_GREY), // 3 storeys (+1 storey!)
        2 => (30, 14, pal::BRICK_RED, pal::ROOF_COPPER_PATINA), // red brick + stone balustrade
        _ => (32, 12, pal::STONE_GRANITE, pal::METAL_STEEL_DARK), // modern glass wing
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + rise, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    parts::roof_hip_tile(sprite, light, -0.05, -0.05, w + 0.05, d + 0.05, z_wall, rise, roof_c, ox, oy, age);

    // Arched reading room windows on both faces
    let len = rf.top_len();
    let wo = WinOpts {
        w: 7,
        h: 12,
        frame: pal::STONE_LIMESTONE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.80,
    };
    parts::draw_window(&rf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, 18, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 24, 4, wo, z_wall);
    parts::draw_window(&rf, sprite, light, len - 12, 4, wo, z_wall);

    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, 6, 4, wo, z_wall);
    parts::draw_window(&lf, sprite, light, len_l - 14, 4, wo, z_wall);

    // Carved stone portal entrance
    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 14, 10, 14, pal::METAL_CHAMPAGNE, pal::STONE_LIMESTONE, 0);
}

pub fn draw_fire_station(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.15f32);
    let v = spec.variant % 4;
    let (z_wall, tower_h, num_bays) = match v {
        0 => (30, 22, 2),
        1 => (42, 28, 3), // 3 storeys (+1 storey!) + 3 apparatus bays
        2 => (30, 22, 2), // stone firehouse
        _ => (32, 24, 2), // modern fire & rescue
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall + tower_h, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let brick_c = if v == 2 { pal::STONE_GRANITE } else { pal::BRICK_RED };
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);

    // Corner hose-drying tower
    let (tx, ty, tw, td) = (0.05, 0.05, 0.38, 0.38);
    let (t_lf, t_rf) = box_faces(tx, ty, tx + tw, ty + td, z_wall, z_wall + tower_h, ox, oy);
    parts::paint_brick(&t_lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&t_rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::roof_flat(sprite, light, tx, ty, tx + tw, ty + td, z_wall + tower_h, 3, pal::STONE_LIMESTONE, ox, oy, age);

    // Overhead red apparatus bay doors
    let _len = rf.top_len();
    let bay_w = 12;
    for bay in 0..num_bays {
        let u = 4 + bay * 16;
        let bq = rf.sub_px(u, z_wall - 14, u + bay_w, z_wall, rf.rows() as f32);
        bq.paint(sprite, &mut |_u, v_win, _x, _y| {
            let is_glass = v_win >= 3 && v_win <= 8;
            let c = if is_glass {
                if light.is_night { color::WIN_WARM } else { pal::GLASS_DARK }
            } else {
                rgb(0xc4, 0x24, 0x24) // bright fire engine red
            };
            Some((color::surf(c, light, FACE_R), (z_wall - v_win) as u16))
        });
    }

    // Upper dormitory windows on both faces
    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: true,
        shutter_c: pal::SHUTTER_GREEN,
        seed: spec.seed,
        lit_p: 0.85,
    };
    parts::draw_window(&rf, sprite, light, 6, 3, wo, z_wall);
    parts::draw_window(&rf, sprite, light, 22, 3, wo, z_wall);

    let len_l = lf.top_len();
    parts::draw_window(&lf, sprite, light, len_l - 12, 3, wo, z_wall);
}

pub fn draw_hospital(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.90f32, 1.90f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 60, // 5 storeys
        1 => 84, // 7 storeys (+2 storeys!) medical tower
        2 => 60, // red brick pavilion
        _ => 72, // 6 storeys contemporary glass
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let white_c = if v == 2 { pal::BRICK_RED } else { pal::METAL_WHITE };
    parts::paint_stucco(&lf, sprite, light, white_c, FACE_L, age);
    parts::paint_stucco(&rf, sprite, light, white_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    // Red Cross medical emblem on facade
    let len = rf.top_len();
    let cross_u = len / 2;
    let cross_v = 8;
    for dy in -3..=3 {
        for dx in -3..=3 {
            if dx == 0 || dy == 0 {
                let cq = rf.sub_px(cross_u + dx, cross_v + dy, cross_u + dx + 1, cross_v + dy + 1, rf.rows() as f32);
                cq.fill(sprite, rgb(0xdc, 0x24, 0x24));
            }
        }
    }

    // Windows on all floors on both faces
    let wo = WinOpts {
        w: 6,
        h: 7,
        frame: pal::METAL_STEEL_DARK,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.90, // hospitals lit 24/7
    };
    let num_floors = z_wall / 12;
    for floor in 0..num_floors {
        let v_win = 3 + floor * 11;
        for bay in 0..5 {
            let u = 4 + bay * (len - 8) / 4;
            if !(floor == 0 && bay == 2) {
                parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
            }
        }
        let len_l = lf.top_len();
        for bay in 0..4 {
            let u = 4 + bay * (len_l - 8) / 3;
            parts::draw_window(&lf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
    }

    // Emergency ambulance canopy
    let can = box_top(w * 0.35, d, w * 0.65, d + 0.35, 14, ox, oy);
    can.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));

    // Rooftop emergency helipad
    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 3, pal::METAL_STEEL_DARK, ox, oy, 0.0);
    let (h_x, h_y) = project(w * 0.75, d * 0.5, z_wall as f32, ox, oy);
    let pad_c = if light.is_night { color::NEON_AMBER } else { color::GOLD_SIGN };
    for dy in -3i32..=3i32 {
        for dx in -3i32..=3i32 {
            let is_h = (dx == -2 || dx == 2 || dy == 0) && dy.abs() <= 2;
            if is_h {
                sprite.set_pixel(h_x.round() as i32 + dx, h_y.round() as i32 + dy, pad_c, z_wall as u16);
            }
        }
    }
}
