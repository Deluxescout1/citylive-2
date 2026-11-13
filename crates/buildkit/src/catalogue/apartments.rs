//! Apartments: 8 visibly distinct designs (brick colours, bay windows, balconies, porticos, roof variety).

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{box_faces, box_top};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

/// 1. Classic Pre-War Brick Walkup with zigzagging fire escape stairs
pub fn draw_brick_walkup(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    let z_wall = match v {
        0 => 52, // 4 storeys
        1 => 64, // 5 storeys (+1 storey!)
        2 => 52,
        _ => 40, // 3 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let brick_c = color::seed_brick_color(spec.seed);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed, ox, oy);

    // Zigzag fire escape ladder on front facade
    let len = rf.top_len();
    let fe_c = color::surf(pal::WROUGHT_IRON, light, 1.0);
    let num_floors = if z_wall > 60 { 5 } else if z_wall > 45 { 4 } else { 3 };
    for floor in 1..num_floors {
        let v_win = 2 + floor * 11;
        let pq = rf.sub_px(len - 15, v_win + 7, len - 3, v_win + 9, rf.rows() as f32);
        pq.fill(sprite, fe_c);
    }

    // Windows on both faces
    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.68,
    };
    for floor in 0..num_floors {
        let v_win = 2 + floor * 11;
        parts::draw_window(&rf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        if floor < num_floors - 1 {
            parts::draw_window(&rf, sprite, light, len - 11, v_win, wo, z_wall - floor * 11);
        }

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 5, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 11, v_win, wo, z_wall - floor * 11);
    }

    parts::draw_door(&rf, sprite, light, len - 11, z_wall - 12, 8, 12, pal::STONE_BROWNSTONE, pal::STONE_LIMESTONE, 0);
}

/// 2. Victorian Brownstone Rowhouse with tri-bay bowed front and high stoop
pub fn draw_brownstone(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    let z_wall = match v {
        0 => 50, // 4 storeys
        1 => 62, // 5 storeys (+1 storey!)
        2 => 50,
        _ => 38, // 3 storeys
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let stone_c = pal::STONE_BROWNSTONE;
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, stone_c, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 5, stone_c, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 1, ox, oy);

    let len = rf.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: stone_c,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.70,
    };
    let num_floors = if z_wall > 55 { 5 } else if z_wall > 45 { 4 } else { 3 };
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 11;
        parts::draw_window(&rf, sprite, light, 6, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&rf, sprite, light, len - 12, v_win, wo, z_wall - floor * 11);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 11);
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 13, 8, 13, pal::TRIM_CREAM, stone_c, 0);
}

/// 3. Modern Mid-Rise Apartment with glass balconies and lobby marquee
pub fn draw_modern_midrise(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 68, // 6 storeys
        1 => 90, // 8 storeys (+2 storeys!)
        2 => 68,
        _ => 80, // 7 storeys
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let accent_c = match v {
        0 => rgb(0x38, 0x76, 0xb8),
        1 => rgb(0xd4, 0x82, 0x36),
        2 => rgb(0x36, 0xa4, 0x6e),
        _ => rgb(0xa4, 0x38, 0x38),
    };

    parts::paint_glass_curtain(&lf, sprite, light, 9, 8, FACE_L, spec.seed + 15);
    parts::paint_glass_curtain(&rf, sprite, light, 9, 8, FACE_R, spec.seed);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    let len = rf.top_len();
    let num_floors = z_wall / 11;
    for floor in 1..num_floors - 1 {
        let v_win = 2 + floor * 11;
        let u_balcony = if (floor % 2) == 0 { 4 } else { len - 16 };
        let bq = rf.sub_px(u_balcony, v_win + 6, u_balcony + 12, v_win + 10, rf.rows() as f32);
        bq.fill(sprite, color::surf(accent_c, light, 1.0));
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 3, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 2, ox, oy);

    let can = box_top(w * 0.30, d, w * 0.70, d + 0.30, 13, ox, oy);
    can.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, FACE_TOP));
}

/// 4. Courtyard Apartment with limestone stringcourses and arched portal
pub fn draw_courtyard(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 58, // 5 storeys
        1 => 70, // 6 storeys (+1 storey!)
        2 => 46, // 4 storeys
        _ => 58,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let brick_c = color::seed_brick_color(spec.seed);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    let len = rf.top_len();
    let num_floors = z_wall / 11;
    for floor in 0..num_floors {
        let v_win = (floor + 1) * 11;
        let sq = rf.sub_px(0, v_win, len, v_win + 1, rf.rows() as f32);
        sq.fill(sprite, color::surf(pal::STONE_LIMESTONE, light, FACE_TOP));
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 3, ox, oy);

    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 11;
        for bay in 0..4 {
            let u = 4 + bay * (len - 8) / 3;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
        let len_l = lf.top_len();
        for bay in 0..3 {
            let u = 4 + bay * (len_l - 8) / 2;
            parts::draw_window(&lf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 13, 10, 13, pal::METAL_STEEL_DARK, pal::STONE_LIMESTONE, 0);
}

/// 5. Tenement with Projecting Bay Windows
pub fn draw_bay_window_tenement(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 54, // 5 storeys
        1 => 65, // 6 storeys
        2 => 44, // 4 storeys
        _ => 54,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let brick_c = color::seed_brick_color(spec.seed);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 4, ox, oy);

    // Projecting multi-sided bay windows on right face
    let len = rf.top_len();
    let num_floors = z_wall / 11;
    for floor in 0..num_floors - 1 {
        let v_win = 3 + floor * 11;
        // Projecting bay box
        let bq = rf.sub_px(6, v_win, len - 6, v_win + 8, rf.rows() as f32);
        bq.fill(sprite, color::surf(color::shade(brick_c, 1.15), light, FACE_R));

        let wo = WinOpts {
            w: 5,
            h: 7,
            frame: pal::TRIM_WHITE,
            sill: true,
            lintel: true,
            shutters: false,
            shutter_c: pal::SHUTTER_BLACK,
            seed: spec.seed,
            lit_p: 0.70,
        };
        parts::draw_window(&rf, sprite, light, 8, v_win + 1, wo, z_wall - floor * 11);
        parts::draw_window(&rf, sprite, light, len - 13, v_win + 1, wo, z_wall - floor * 11);
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 4, z_wall - 12, 8, 12, pal::STONE_BROWNSTONE, pal::STONE_LIMESTONE, 0);
}

/// 6. Corner Balcony Apartment Building with ground floor retail
pub fn draw_corner_balcony(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 68, // 6 storeys
        1 => 80, // 7 storeys
        2 => 58, // 5 storeys
        _ => 68,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let brick_c = color::seed_brick_color(spec.seed);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::METAL_STEEL_DARK, ox, oy);

    // Wrap-around balconies on corner (w, d)
    let len = rf.top_len();
    let num_floors = z_wall / 11;
    for floor in 1..num_floors - 1 {
        let v_win = 2 + floor * 11;
        parts::draw_balcony(&rf, sprite, light, len - 16, v_win + 6, 14, 5, z_wall - v_win);
        parts::draw_balcony(&lf, sprite, light, 3, v_win + 6, 14, 5, z_wall - v_win);
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::METAL_STEEL_DARK, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed + 5, ox, oy);

    // Ground floor retail shopfront
    parts::draw_storefront(&rf, sprite, light, 4, z_wall - 14, len - 18, 13, pal::METAL_STEEL_DARK, 0, spec.seed);
    parts::draw_door(&rf, sprite, light, len - 12, z_wall - 14, 8, 13, pal::TRIM_WHITE, pal::METAL_STEEL_DARK, 0);
}

/// 7. Art Deco Masonry Apartment Block with vertical fluted piers
pub fn draw_art_deco_masonry(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let z_wall = match v {
        0 => 74, // 7 storeys
        1 => 86, // 8 storeys
        2 => 62, // 6 storeys
        _ => 74,
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    let stone_c = pal::STONE_LIMESTONE;
    parts::paint_stone_ashlar(&lf, sprite, light, stone_c, FACE_L, age);
    parts::paint_stone_ashlar(&rf, sprite, light, stone_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, stone_c, ox, oy);

    // Vertical Art Deco fluted masonry piers
    let len = rf.top_len();
    for col in 0..4 {
        let u = 4 + col * (len - 8) / 3;
        let pier_q = rf.sub_px(u, 0, u + 2, z_wall - 12, rf.rows() as f32);
        pier_q.fill(sprite, color::surf(color::shade(stone_c, 1.15), light, FACE_R));
    }

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 5, stone_c, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z_wall, spec.seed, ox, oy);

    let wo = WinOpts {
        w: 6,
        h: 8,
        frame: pal::METAL_CHAMPAGNE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let num_floors = z_wall / 11;
    for floor in 0..num_floors - 1 {
        let v_win = 2 + floor * 11;
        for bay in 0..3 {
            let u = 7 + bay * (len - 8) / 3;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
    }

    parts::draw_door(&rf, sprite, light, len / 2 - 5, z_wall - 14, 10, 14, pal::METAL_CHAMPAGNE, pal::STONE_GRANITE, 0);
}

/// 8. Terraced Garden Apartment with stepped setback balconies and pergolas
pub fn draw_terrace_garden(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.85f32);
    let v = spec.variant % 4;
    let (z1, z2) = match v {
        0 => (48, 68), // 6 storeys
        1 => (58, 80), // 7 storeys
        2 => (38, 56), // 5 storeys
        _ => (48, 68),
    };
    let age = spec.age;

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z2, ox, oy);

    // Lower tier
    let (lf1, rf1) = box_faces(0.0, 0.0, w, d, 0, z1, ox, oy);
    let stucco_c = pal::STUCCO_SAND;
    parts::paint_stucco(&lf1, sprite, light, stucco_c, FACE_L, age);
    parts::paint_stucco(&rf1, sprite, light, stucco_c, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z1, pal::STONE_LIMESTONE, ox, oy);

    // Landscaped setback terrace on floor 4
    let t1 = box_top(0.0, 0.0, w, d, z1, ox, oy);
    t1.fill(sprite, color::surf(pal::FOLIAGE_GREEN, light, FACE_TOP));

    // Upper tier inset by 0.3
    let (lf2, rf2) = box_faces(0.3, 0.3, w - 0.3, d - 0.3, z1, z2, ox, oy);
    parts::paint_stucco(&lf2, sprite, light, stucco_c, FACE_L, age);
    parts::paint_stucco(&rf2, sprite, light, stucco_c, FACE_R, age);
    parts::corner_seam(sprite, light, w - 0.3, d - 0.3, z1, z2, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.3, 0.3, w - 0.3, d - 0.3, z2, 4, pal::STONE_LIMESTONE, ox, oy, age);
    parts::draw_seed_rooftop_elements(sprite, light, w, d, z2, spec.seed + 1, ox, oy);

    let len = rf1.top_len();
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.70,
    };
    parts::draw_window(&rf1, sprite, light, 6, 4, wo, z1);
    parts::draw_window(&rf1, sprite, light, len - 12, 4, wo, z1);
    parts::draw_door(&rf1, sprite, light, len / 2 - 4, z1 - 12, 8, 12, pal::STONE_BROWNSTONE, pal::TRIM_WHITE, 0);
}
