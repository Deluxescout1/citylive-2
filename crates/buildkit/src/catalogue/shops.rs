//! Brick Main-Street Shops: 2-Storey Shop, 3-Storey Cafe/Bakery, 4-Storey Corner Store, Bookstore Arcade.

use render::palette::rgb;
use crate::color::{self, pal, FACE_L, FACE_R, FACE_TOP};
use crate::geom::{self, box_faces, box_top};
use crate::parts::{self, WinOpts};
use crate::types::{BuildingSpec, Light, Sprite};

pub fn draw_shop_2storey(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, brick_c, awning_a, awning_b, has_blade_sign) = match v {
        0 => (28, pal::BRICK_RED, rgb(0xc8, 0x36, 0x36), pal::TRIM_WHITE, false),
        1 => (42, pal::BRICK_DARK_CLINKER, rgb(0x2e, 0x6e, 0x48), pal::TRIM_CREAM, false), // 3 storeys (+1 storey!)
        2 => (28, pal::BRICK_BUFF, rgb(0x28, 0x48, 0x7e), pal::TRIM_WHITE, true), // 2 storeys + blade sign + pediment
        _ => (30, pal::CLAP_WHITE, rgb(0x28, 0x28, 0x28), pal::TRIM_WHITE, false), // 2 storeys white brick + black awning
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    // Flat roof with parapet
    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);

    // Ground floor storefront on right face
    let len = rf.top_len();
    parts::draw_storefront(&rf, sprite, light, 3, z_wall - 14, len - 16, 13, pal::METAL_STEEL_DARK, 0, spec.seed);
    parts::draw_door(&rf, sprite, light, len - 12, z_wall - 14, 8, 13, pal::TRIM_WHITE, pal::METAL_STEEL_DARK, 0);

    // Striped canvas awning above storefront
    parts::draw_awning(&rf, sprite, light, 2, z_wall - 18, len - 4, 6, awning_a, awning_b, 12);

    // Upper floor sash windows on both faces
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let num_upper = if z_wall > 35 { 2 } else { 1 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 12;
        parts::draw_window(&rf, sprite, light, 6, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&rf, sprite, light, len - 12, v_win, wo, z_wall - floor * 12);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 12);
    }

    // Commercial gold signboard band
    let sign_q = rf.sub_px(3, z_wall - 21, len - 3, z_wall - 18, rf.rows() as f32);
    sign_q.fill(sprite, color::surf(pal::METAL_STEEL_DARK, light, 1.0));

    // Projecting blade sign for v2
    if has_blade_sign {
        let (sign_x, sign_y) = geom::project(w, d, 20.0, ox, oy);
        let sign_c = if light.is_night { color::NEON_AMBER } else { color::GOLD_SIGN };
        for dy in -4..=4 {
            sprite.set_pixel(sign_x.round() as i32 + 1, sign_y.round() as i32 + dy, sign_c, 20);
            sprite.set_pixel(sign_x.round() as i32 + 2, sign_y.round() as i32 + dy, sign_c, 20);
        }
    }
}

pub fn draw_cafe_3storey(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, brick_c, awning_c, is_limestone) = match v {
        0 => (42, pal::BRICK_DARK_CLINKER, rgb(0x8a, 0x24, 0x2a), false),
        1 => (54, pal::BRICK_ORANGE, rgb(0x28, 0x48, 0x6e), false), // 4 storeys (+1 storey!)
        2 => (42, pal::STONE_LIMESTONE, rgb(0x24, 0x24, 0x24), true), // 3 storeys limestone ashlar
        _ => (30, pal::STONE_BROWNSTONE, rgb(0x2e, 0x6e, 0x48), false), // 2 storeys brownstone + copper cornice
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    if is_limestone {
        parts::paint_stone_ashlar(&lf, sprite, light, brick_c, FACE_L, age);
        parts::paint_stone_ashlar(&rf, sprite, light, brick_c, FACE_R, age);
    } else {
        parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
        parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    }
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_GRANITE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 5, pal::STONE_GRANITE, ox, oy, age);

    // Ground floor cafe display
    let len = rf.top_len();
    parts::draw_storefront(&rf, sprite, light, 3, z_wall - 14, len - 14, 13, pal::TRIM_CREAM, 0, spec.seed);
    parts::draw_door(&rf, sprite, light, len - 10, z_wall - 14, 8, 13, pal::METAL_CHAMPAGNE, pal::TRIM_CREAM, 0);

    // Awning over cafe
    parts::draw_awning(&rf, sprite, light, 2, z_wall - 19, len - 3, 6, awning_c, pal::TRIM_CREAM, 12);

    // Upper floor windows with arched keystones
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_CREAM,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_GREEN,
        seed: spec.seed,
        lit_p: 0.65,
    };
    let num_upper = if z_wall > 45 { 3 } else if z_wall > 35 { 2 } else { 1 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 11;
        parts::draw_window(&rf, sprite, light, 6, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&rf, sprite, light, len - 12, v_win, wo, z_wall - floor * 11);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 11);
    }

    // Glowing neon blade sign on corner
    let (sign_x, sign_y) = geom::project(w, d, 28.0, ox, oy);
    let neon_c = color::NEON_PINK;
    if light.is_night {
        for dy in -4..=4 {
            sprite.set_pixel(sign_x.round() as i32 + 1, sign_y.round() as i32 + dy, neon_c, 28);
        }
    }
}

pub fn draw_corner_store_4storey(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (2.05f32, 1.10f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, brick_c, has_billboard, has_water_tower) = match v {
        0 => (54, pal::BRICK_RED, true, false), // 4 storeys + illuminated billboard
        1 => (66, pal::BRICK_BUFF, false, false), // 5 storeys (+1 storey!)
        2 => (42, pal::BRICK_DARK_CLINKER, false, false), // 3 storeys + corner clock
        _ => (54, pal::CLAP_SAGE, false, true), // 4 storeys sage brick + fire escape + water tower
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 5, pal::STONE_LIMESTONE, ox, oy, age);

    // Rooftop illuminated billboard for v0
    if has_billboard {
        let (_bb_lf, bb_rf) = box_faces(w * 0.2, d * 0.2, w * 0.8, d * 0.5, z_wall + 5, z_wall + 20, ox, oy);
        bb_rf.paint(sprite, &mut |u, v_bb, _x, _y| {
            let depth = (z_wall + 20) as u16;
            if light.is_night {
                let lit = (u % 3 == 0) && (v_bb % 2 == 0);
                let c = if lit { color::NEON_CYAN } else { color::NEON_PINK };
                Some((c, depth))
            } else {
                let c = if (u + v_bb) % 4 == 0 { pal::METAL_STEEL_DARK } else { pal::METAL_WHITE };
                Some((color::surf(c, light, FACE_R), depth))
            }
        });
    }

    // Rooftop water tower for v3
    if has_water_tower {
        let (cx, cy) = (w * 0.5, d * 0.3);
        let (w_lf, w_rf) = box_faces(cx, cy, cx + 0.35, cy + 0.35, z_wall + 8, z_wall + 20, ox, oy);
        w_lf.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_L));
        w_rf.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_R));
        let w_top = box_top(cx, cy, cx + 0.35, cy + 0.35, z_wall + 20, ox, oy);
        w_top.fill(sprite, color::surf(pal::ROOF_CEDAR_SHAKE, light, FACE_TOP));
    }

    // Windows on upper floors
    let wo = WinOpts {
        w: 6,
        h: 9,
        frame: pal::TRIM_WHITE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.60,
    };
    let len = rf.top_len();
    let num_upper = if z_wall > 60 { 4 } else if z_wall > 45 { 3 } else { 2 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 11;
        for bay in 0..4 {
            let u = 4 + bay * (len - 8) / 3;
            parts::draw_window(&rf, sprite, light, u, v_win, wo, z_wall - floor * 11);
        }
        // Left face windows
        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 6, v_win, wo, z_wall - floor * 11);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 11);
    }

    // Ground floor commercial wrap
    parts::draw_storefront(&rf, sprite, light, 4, z_wall - 14, len - 20, 13, pal::METAL_STEEL_DARK, 0, spec.seed);
    parts::draw_door(&rf, sprite, light, len - 14, z_wall - 14, 8, 13, pal::TRIM_WHITE, pal::METAL_STEEL_DARK, 0);
}

pub fn draw_bookstore_arcade(spec: &BuildingSpec, light: &Light, sprite: &mut Sprite, ox: f32, oy: f32) {
    let (w, d) = (1.15f32, 1.05f32);
    let v = spec.variant % 4;
    let age = spec.age;

    // Structural variation across 4 variants
    let (z_wall, has_cupola, has_mansard) = match v {
        0 => (42, true, false), // 3 storeys + copper cupola
        1 => (54, false, true), // 4 storeys + Venetian Gothic slate mansard
        2 => (42, false, false), // 3 storeys + red brick / sandstone banding
        _ => (30, false, false), // 2 storeys + wide double awning + stone entablature
    };

    parts::building_shadow(sprite, 0.0, 0.0, w, d, z_wall, ox, oy);

    let brick_c = pal::BRICK_RED;
    let (lf, rf) = box_faces(0.0, 0.0, w, d, 0, z_wall, ox, oy);
    parts::paint_brick(&lf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_L, age);
    parts::paint_brick(&rf, sprite, light, brick_c, pal::BRICK_MORTAR, FACE_R, age);
    parts::corner_seam(sprite, light, w, d, 0, z_wall, pal::STONE_LIMESTONE, ox, oy);

    if has_mansard {
        parts::roof_mansard(sprite, light, 0.0, 0.0, w, d, z_wall, 14, pal::ROOF_SLATE_GREY, ox, oy, age);
    } else {
        parts::roof_flat(sprite, light, 0.0, 0.0, w, d, z_wall, 4, pal::STONE_LIMESTONE, ox, oy, age);
    }

    // Central roof copper cupola with flagpole for v0
    if has_cupola {
        let (cx0, cy0, cw, cd) = (w / 2.0 - 0.12, d / 2.0 - 0.12, 0.24, 0.24);
        let (cup_lf, cup_rf) = box_faces(cx0, cy0, cx0 + cw, cy0 + cd, z_wall + 4, z_wall + 14, ox, oy);
        cup_lf.fill(sprite, color::surf(pal::ROOF_COPPER_PATINA, light, FACE_L));
        cup_rf.fill(sprite, color::surf(color::shade(pal::ROOF_COPPER_PATINA, 0.8), light, FACE_R));
        parts::spire_needle(sprite, light, w / 2.0, d / 2.0, z_wall + 14, 12, ox, oy);
    }

    // Double Romanesque arched windows on upper floors
    let len = rf.top_len();
    let wo = WinOpts {
        w: 7,
        h: 10,
        frame: pal::STONE_LIMESTONE,
        sill: true,
        lintel: true,
        shutters: false,
        shutter_c: pal::SHUTTER_BLACK,
        seed: spec.seed,
        lit_p: 0.70,
    };
    let num_upper = if z_wall > 45 { 3 } else if z_wall > 35 { 2 } else { 1 };
    for floor in 0..num_upper {
        let v_win = 3 + floor * 12;
        parts::draw_window(&rf, sprite, light, 4, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&rf, sprite, light, len - 12, v_win, wo, z_wall - floor * 12);

        let len_l = lf.top_len();
        parts::draw_window(&lf, sprite, light, 4, v_win, wo, z_wall - floor * 12);
        parts::draw_window(&lf, sprite, light, len_l - 12, v_win, wo, z_wall - floor * 12);
    }

    // Ground floor bookstore entrance
    parts::draw_storefront(&rf, sprite, light, 3, z_wall - 14, len - 13, 13, pal::METAL_STEEL_DARK, 0, spec.seed);
    parts::draw_door(&rf, sprite, light, len - 9, z_wall - 14, 7, 13, pal::TRIM_WHITE, pal::METAL_STEEL_DARK, 0);
}
